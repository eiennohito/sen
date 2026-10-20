slint::include_modules!();

impl From<i32> for NotificationPreview {
    fn from(value: i32) -> Self {
        match value {
            0 => Self::NameAndText,
            1 => Self::NameOnly,
            _ => Self::NewMessage,
        }
    }
}
impl From<NotificationPreview> for i32 {
    fn from(value: NotificationPreview) -> Self {
        match value {
            NotificationPreview::NameAndText => 0,
            NotificationPreview::NameOnly => 1,
            NotificationPreview::NewMessage => 2,
        }
    }
}
