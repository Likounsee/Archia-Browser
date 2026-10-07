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
    #[cfg(any(target_os = "archiaos", target_os = "none"))]
    {
        Platform::ArchiaOs
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "archiaos",
        target_os = "none"
    )))]
    {
        Platform::Linux
    }
}
