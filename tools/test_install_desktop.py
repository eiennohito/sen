from pathlib import Path
from unittest.mock import patch
import contextlib, io, shutil, sys, tempfile
root=Path(__file__).resolve().parent.parent
sys.path.insert(0,str(root/'tools'))
source=(root/'tools/install-desktop.py').read_text()
real_is_file=Path.is_file
real_copyfile=shutil.copyfile
binary=root/'target/release/sen'
def is_file(path):
    return path==binary or real_is_file(path)
def copyfile(src,dst,*args,**kwargs):
    if Path(src)==binary:
        Path(dst).write_bytes(b'synthetic executable, never run')
        return dst
    return real_copyfile(src,dst,*args,**kwargs)
def install(stage):
    with patch.object(sys,'argv',['install-desktop.py','--destdir',str(stage)]), patch.object(Path,'is_file',is_file), patch('shutil.copyfile',copyfile), patch('subprocess.check_output',return_value=b'{"demo":false,"mcp_debug":false}'), contextlib.redirect_stdout(io.StringIO()):
        exec(compile(source,str(root/'tools/install-desktop.py'),'exec'),{'__file__':str(root/'tools/install-desktop.py'),'__name__':'__main__'})
with tempfile.TemporaryDirectory(dir=root/'target/tmp',prefix='installer-fix-') as temp:
    stage=Path(temp)
    launch=stage/'share/applications/org.lineclient.Desktop.desktop'
    service=stage/'share/dbus-1/services/org.lineclient.Desktop.service'
    install(stage)
    for marker in ['X-Sen-Managed','X-LINE-Managed']:
        launch.write_text(f'[Desktop Entry]\n{marker}=true\n')
        service.write_text(f'[D-BUS Service]\n{marker}=true\n')
        install(stage)
        assert not launch.exists(), 'managed launcher remains'
        assert not service.exists(), 'managed activation remains'
    # The previous installer emitted no service marker; recognise its exact command.
    import shlex
    service.write_text('[D-BUS Service]\nName=org.lineclient.Desktop\nExec='+shlex.quote(str(stage/'bin/line-desktop'))+'\n')
    install(stage)
    assert not service.exists(), 'original activation remains'
    custom_service='[D-BUS Service]\nName=org.lineclient.Desktop\nExec=/custom/line-desktop\n'
    service.write_text(custom_service)
    # An unmarked launcher emitted by the original installer stays too.
    new=(stage/'share/applications/org.senclient.Sen.desktop').read_text()
    command='/usr/bin/env -- '+ '"'+str(stage/'bin/sen')+'"'
    old_command='/usr/bin/env -- '+ '"'+str(stage/'bin/line-desktop')+'"'
    old_icon=str((stage/'share/icons/hicolor/scalable/apps/org.lineclient.Desktop.svg').resolve())
    new_icon=str((stage/'share/icons/hicolor/scalable/apps/org.senclient.Sen.svg').resolve())
    unmarked=new.replace(command,old_command).replace(new_icon,old_icon).replace('org.senclient.Sen','org.lineclient.Desktop').replace('X-Sen-Managed=true\n','')
    launch.write_text(unmarked)
    install(stage)
    assert launch.exists(), 'unmarked launcher removed'
    assert launch.read_text()==unmarked, 'unmarked launcher changed'
    assert service.read_text()==custom_service, 'custom activation removed'
print('managed and original activation removed; custom activation and unmarked launcher kept')
