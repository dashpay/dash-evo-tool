/// Load some resource that is bundled with the application.
///
/// Supported paths:
/// - `.env.example`: Loads the bundled `.env.example` file.
pub(crate) enum BundledResource {
    DotEnvExample,
    CoreConfigMainnet,
    CoreConfigTestnet,
    CoreConfigDevnet,
}

impl BundledResource {
    /// Loads the resource as a byte slice.
    pub(crate) fn load(&self) -> &'static [u8] {
        match self {
            Self::DotEnvExample => include_bytes!("../.env.example"),
            Self::CoreConfigMainnet => include_bytes!("../dash_core_configs/mainnet.conf"),
            Self::CoreConfigTestnet => include_bytes!("../dash_core_configs/testnet.conf"),
            Self::CoreConfigDevnet => include_bytes!("../dash_core_configs/devnet.conf"),
        }
    }

    /// Writes the resource to a file. Creates directories if they do not exist.
    /// When overwriting, it will replace the file if it exists.
    ///
    /// Returns `Ok(true)` if the file was written, `Ok(false)` if it already existed and was not overwritten,
    /// or an `io::Error` if there was an issue writing the file.
    pub(crate) fn write_to_file(
        &self,
        path: &std::path::Path,
        overwrite: bool,
    ) -> std::io::Result<bool> {
        let exists = path.exists();
        if !exists || overwrite {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, self.load())?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
