#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    ArchiaOs,
}

pub fn current() -> Platform {
    #[cfg(target_os = "windows")]
    {
        Platform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        Platform::Linux
    }
    #[cfg(target_os = "archiaos")]
    {
        Platform::ArchiaOs
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "archiaos")))]
    {
        Platform::Linux
    }
}
