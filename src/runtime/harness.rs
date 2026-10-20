use super::*;

impl Runtime {
    pub fn bindings(&self) -> Vec<(String, Vec<String>)> {
        self.store
            .data()
            .classes
            .values()
            .map(|c| {
                (
                    c.name.clone(),
                    c.methods.keys().chain(c.statics.keys()).cloned().collect(),
                )
            })
            .collect()
    }
    /// Read immutable module data for the synthetic opaque-ID test helper.
    pub fn read_memory(&self, address: usize, len: usize) -> Result<Vec<u8>> {
        let mem = self
            .instance
            .get_memory(&self.store, "O")
            .context("missing memory")?;
        let end = address.checked_add(len).context("memory range overflow")?;
        Ok(mem
            .data(&self.store)
            .get(address..end)
            .context("memory read bounds")?
            .to_vec())
    }
}
