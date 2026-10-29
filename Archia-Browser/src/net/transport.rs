use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::{CancellationToken, Request, Response, Transport, TransportError};

const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_REQUEST_HEADER_COUNT: usize = 256;
const MAX_RESPONSE_HEADER_COUNT: usize = 256;
const MAX_INTERIM_RESPONSES: usize = 16;

/// Minimal HTTP/1.1 transport with certificate-validated TLS for HTTPS origins.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    connect_timeout: Duration,
    read_timeout: Duration,
    max_response_size: usize,
    max_header_size: usize,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    pub const fn new() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(20),
            max_response_size: 8 * 1024 * 1024,
            max_header_size: 64 * 1024,
        }
    }

    pub const fn with_timeouts(connect_timeout: Duration, read_timeout: Duration) -> Self {
        Self {
            connect_timeout,
            read_timeout,
            max_response_size: 8 * 1024 * 1024,
            max_header_size: 64 * 1024,
        }
    }

    pub const fn with_limits(mut self, max_response_size: usize, max_header_size: usize) -> Self {
        self.max_response_size = max_response_size;
        self.max_header_size = max_header_size;
        self
    }

    fn connect(
        &self,
        request: &Request,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn ReadWrite>, TransportError> {
        cancellation.check()?;
        match request.url.scheme() {
            "http" | "https" => {}
            _ => return Err(TransportError::UnsupportedScheme),
        }

        // Validate the TLS identity before DNS lookup or opening a socket.
        // Invalid SNI must fail locally, without causing network side effects.
        let server_name = if request.url.scheme() == "https" {
            Some(tls_server_name(request.url.host())?)
        } else {
            None
        };

        let address = socket_address(&request.url);
        let addresses = address
            .to_socket_addrs()
            .map_err(|_| TransportError::ConnectionFailed)?;
        let mut connected = None;
        let mut connect_timed_out = false;
        for address in addresses {
            cancellation.check()?;
            match TcpStream::connect_timeout(&address, self.connect_timeout) {
                Ok(stream) => {
                    connected = Some(stream);
                    break;
                }
                Err(error) => {
                    connect_timed_out |= is_timeout_error(error.kind());
                }
            }
        }
        let mut stream = connected.ok_or_else(|| connect_failure_error(connect_timed_out))?;

        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(self.read_timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;

        if request.url.scheme() == "http" {
            return Ok(Box::new(stream));
        }

        let config = tls_client_config();
        let server_name = server_name.ok_or(TransportError::TlsFailed)?;
        let mut connection = ClientConnection::new(Arc::new(config), server_name)
            .map_err(|_| TransportError::TlsFailed)?;
        // Complete the handshake here so certificate and protocol failures are
        // reported as TLS errors, rather than being misclassified as HTTP I/O.
        while connection.is_handshaking() {
            cancellation.check()?;
            connection
                .complete_io(&mut stream)
                .map_err(map_tls_io_error)?;
        }
        Ok(Box::new(StreamOwned::new(connection, stream)))
    }

    fn write_request(
        &self,
        stream: &mut dyn ReadWrite,
        request: &Request,
    ) -> Result<(), TransportError> {
        validate_request(request)?;
        let mut head = String::new();
        head.push_str(request.method.as_str());
        head.push(' ');
        head.push_str(request.url.path());
        if let Some(query) = request.url.query() {
            head.push('?');
            head.push_str(query);
        }
        head.push_str(" HTTP/1.1\r\n");

        if !request
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("host"))
        {
            head.push_str("Host: ");
            head.push_str(&host_header(request));
            head.push_str("\r\n");
        }
        // The response reader consumes until EOF. Force connection-close
        // semantics rather than allowing a caller-supplied keep-alive header
        // to make an otherwise complete response wait until the read timeout.
        head.push_str("Connection: close\r\n");
        if request.has_body()
            && !request
                .headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("content-length"))
        {
            head.push_str("Content-Length: ");
            head.push_str(&request.body.len().to_string());
            head.push_str("\r\n");
        }

        for (name, value) in &request.headers {
            if name.eq_ignore_ascii_case("connection") {
                continue;
            }
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");

        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.write_all(&request.body))
            .map_err(map_transport_io_error)?;
        Ok(())
    }

    fn read_response(
        &self,
        stream: &mut dyn ReadWrite,
        method: super::HttpMethod,
        cancellation: &CancellationToken,
    ) -> Result<Response, TransportError> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        let mut header_end = None;

        loop {
            cancellation.check()?;
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    // Check the projected total before growing the response buffer.
                    // Checking only after extend_from_slice can temporarily allocate
                    // beyond the configured cap by one complete read chunk.
                    let Some(projected_len) = bytes.len().checked_add(count) else {
                        return Err(TransportError::ResponseTooLarge);
                    };
                    if projected_len > self.max_response_size {
                        return Err(TransportError::ResponseTooLarge);
                    }
                    if bytes.try_reserve_exact(count).is_err() {
                        return Err(TransportError::ResponseTooLarge);
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    if header_end.is_none() {
                        header_end = bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|position| position + 4);
                    }
                    // Enforce the header limit as soon as the delimiter arrives.
                    // A single read can cross the limit and contain the delimiter;
                    // checking only while it is absent would otherwise defer rejection
                    // until the entire response has been buffered.
                    if let Some(end) = header_end {
                        if end.saturating_sub(4) > self.max_header_size {
                            return Err(TransportError::ResponseTooLarge);
                        }
                    } else if bytes.len() > self.max_header_size.saturating_add(3) {
                        // Up to three bytes beyond the limit may be a partial
                        // CRLFCRLF delimiter. Allow that suffix across reads,
                        // while bounding an unterminated header to limit + 3.
                        return Err(TransportError::ResponseTooLarge);
                    }
                    if bytes.len() > self.max_response_size {
                        return Err(TransportError::ResponseTooLarge);
                    }
                }
                Err(error) => return Err(map_transport_io_error(error)),
            }
        }

        if bytes.len() > self.max_response_size {
            return Err(TransportError::ResponseTooLarge);
        }
        parse_http_response_for_method(&bytes, method, self.max_response_size, self.max_header_size)
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn tls_client_config() -> ClientConfig {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    // This transport only parses HTTP/1.1. Never negotiate a protocol it
    // cannot decode, and make the supported application protocol explicit.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

fn map_transport_io_error(error: std::io::Error) -> TransportError {
    if is_timeout_error(error.kind()) {
        TransportError::Timeout
    } else {
        TransportError::ConnectionFailed
    }
}

fn is_timeout_error(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn connect_failure_error(timed_out: bool) -> TransportError {
    if timed_out {
        TransportError::Timeout
    } else {
        TransportError::ConnectionFailed
    }
}

fn map_tls_io_error(error: std::io::Error) -> TransportError {
    if is_timeout_error(error.kind()) {
        TransportError::Timeout
    } else {
        TransportError::TlsFailed
    }
}

fn tls_server_name(host: &str) -> Result<ServerName<'static>, TransportError> {
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return Ok(ServerName::IpAddress(address.into()));
    }
    ServerName::try_from(host.to_owned()).map_err(|_| TransportError::TlsFailed)
}

impl Transport for HttpTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.send_cancellable(request, &CancellationToken::new())
    }

    fn send_cancellable(
        &self,
        request: &Request,
        cancellation: &CancellationToken,
    ) -> Result<Response, TransportError> {
        cancellation.check()?;
        validate_request(request)?;
        let mut stream = self.connect(request, cancellation)?;
        cancellation.check()?;
        self.write_request(&mut stream, request)?;
        cancellation.check()?;
        self.read_response(&mut stream, request.method, cancellation)
    }
}

#[derive(Debug, Clone)]
pub struct LocalFileTransport {
    max_file_size: usize,
    allowed_roots: Vec<std::path::PathBuf>,
    #[cfg(any(unix, windows))]
    authorized_root_handles: Vec<AuthorizedRootHandle>,
}

#[cfg(any(unix, windows))]
#[derive(Debug, Clone)]
struct AuthorizedRootHandle {
    canonical_path: std::path::PathBuf,
    directory: Arc<std::fs::File>,
}

impl Default for LocalFileTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalFileTransport {
    pub const fn new() -> Self {
        Self {
            max_file_size: 8 * 1024 * 1024,
            // Fail closed until the embedding application explicitly grants
            // access to one or more local directories.
            allowed_roots: Vec::new(),
            #[cfg(any(unix, windows))]
            authorized_root_handles: Vec::new(),
        }
    }

    /// Restrict file URLs to the supplied directories. Roots must exist and
    /// be directories; an empty list denies all local-file reads.
    pub fn with_allowed_roots(
        mut self,
        roots: impl IntoIterator<Item = std::path::PathBuf>,
    ) -> Self {
        self.allowed_roots = roots.into_iter().collect();
        #[cfg(any(unix, windows))]
        {
            self.authorized_root_handles = self
                .allowed_roots
                .iter()
                .filter_map(|root| {
                    // Check the caller-supplied spelling before canonicalization:
                    // canonicalize() follows a symlink at the selected root and
                    // would otherwise erase evidence that the grant path itself
                    // was redirected. Check the resolved path too, so both the
                    // requested path and the resulting directory must be clean.
                    reject_symlink_path_components(root).ok()?;

                    // Pin the caller-supplied directory before canonicalizing
                    // on Unix. This preserves the original directory object if
                    // the path changes during canonicalization; compare its
                    // identity with the resolved name before accepting the grant.
                    #[cfg(any(unix, windows))]
                    let directory = open_local_root_handle_verified(root).ok()?;

                    let canonical_path = std::fs::canonicalize(root).ok()?;
                    reject_symlink_path_components(&canonical_path).ok()?;
                    if !canonical_path.is_dir() {
                        return None;
                    }

                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;

                        let opened = directory.metadata().ok()?;
                        let named = std::fs::symlink_metadata(&canonical_path).ok()?;
                        if !named.is_dir()
                            || named.file_type().is_symlink()
                            || opened.dev() != named.dev()
                            || opened.ino() != named.ino()
                        {
                            return None;
                        }
                    }

                    #[cfg(windows)]
                    {
                        // Compare the original grant handle against the resolved
                        // path using stable Win32 file identity, not unstable Rust APIs.
                        let resolved = open_local_root_handle_verified(&canonical_path).ok()?;
                        if windows_file_identity(&directory).is_none()
                            || windows_file_identity(&directory) != windows_file_identity(&resolved)
                        {
                            return None;
                        }
                    }

                    Some(AuthorizedRootHandle {
                        canonical_path,
                        directory: Arc::new(directory),
                    })
                })
                .collect();
        }
        self
    }

    /// Create a transport scoped to the directory containing an explicitly
    /// selected local-file URL. Sibling resources are allowed; other paths are
    /// denied by the same root-confinement checks used by `with_allowed_roots`.
    pub fn for_file_url(url: &super::Url) -> Result<Self, TransportError> {
        if url.scheme() != "file" {
            return Err(TransportError::UnsupportedScheme);
        }
        let path = file_url_path(url.path())?;
        let root = path
            .parent()
            .ok_or(TransportError::InvalidRequest)?
            .to_path_buf();
        Ok(Self::new().with_allowed_roots([root]))
    }

    pub const fn with_max_file_size(mut self, max_file_size: usize) -> Self {
        self.max_file_size = max_file_size;
        self
    }
}

impl Transport for LocalFileTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        if request.url.scheme() != "file"
            || !request.url.authority().is_empty()
                && !request.url.authority().eq_ignore_ascii_case("localhost")
        {
            return Err(TransportError::UnsupportedScheme);
        }
        if !matches!(
            request.method,
            super::HttpMethod::Get | super::HttpMethod::Head
        ) || request.has_body()
        {
            return Err(TransportError::InvalidRequest);
        }

        let path = file_url_path(request.url.path())?;
        // No local files are accessible until the embedding application
        // explicitly configures allowed roots.
        if self.allowed_roots.is_empty() {
            return Err(TransportError::ConnectionFailed);
        }

        // Pathname checks are defense in depth. The actual open must be anchored
        // to the pinned grant handle to prevent concurrent symlink/rename swaps.
        reject_symlink_path_components(&path)?;
        let canonical_path =
            std::fs::canonicalize(&path).map_err(|_| TransportError::ConnectionFailed)?;
        let mut authorized_root = None;
        for root in &self.allowed_roots {
            // A grant can become unavailable after configuration (for example,
            // a removable directory disappears). Skip that grant and continue
            // checking the remaining pinned grants instead of making one stale
            // entry disable access to every other authorized root.
            let Ok(canonical_root) = std::fs::canonicalize(root) else {
                continue;
            };
            // A root that has become a symlink or otherwise fails validation
            // is no longer a usable grant. Fail closed for that root, not for
            // unrelated roots in the same allowlist.
            let root_is_valid =
                reject_symlink_path_components(&canonical_root).is_ok() && canonical_root.is_dir();
            if !root_is_valid {
                continue;
            }
            if canonical_path.starts_with(&canonical_root) {
                authorized_root = Some(canonical_root);
                break;
            }
        }
        let canonical_root = authorized_root.ok_or(TransportError::ConnectionFailed)?;
        // Open relative to the directory handle captured when access was granted.
        // Linux uses openat2; other Unix platforms walk each component with openat
        // and O_NOFOLLOW, so a concurrent parent swap cannot escape the root.
        #[cfg(target_os = "linux")]
        let file = {
            let pinned_root = self
                .authorized_root_handles
                .iter()
                .find(|root| {
                    canonical_path.starts_with(&root.canonical_path)
                        && root.canonical_path == canonical_root
                })
                .ok_or(TransportError::ConnectionFailed)?;
            open_local_file_confined_from_handle(
                &pinned_root.directory,
                &pinned_root.canonical_path,
                &canonical_path,
            )?
        };
        #[cfg(all(unix, not(target_os = "linux")))]
        let file = {
            let pinned_root = self
                .authorized_root_handles
                .iter()
                .find(|root| {
                    canonical_path.starts_with(&root.canonical_path)
                        && root.canonical_path == canonical_root
                })
                .ok_or(TransportError::ConnectionFailed)?;
            open_local_file_confined_from_handle(
                &pinned_root.directory,
                &pinned_root.canonical_path,
                &canonical_path,
            )?
        };
        #[cfg(windows)]
        let file = {
            let pinned_root = self
                .authorized_root_handles
                .iter()
                .find(|root| {
                    canonical_path.starts_with(&root.canonical_path)
                        && root.canonical_path == canonical_root
                })
                .ok_or(TransportError::ConnectionFailed)?;
            open_local_file_confined_from_handle(
                &pinned_root.directory,
                &pinned_root.canonical_path,
                &canonical_path,
            )?
        };
        #[cfg(not(any(unix, windows)))]
        let file = open_local_file_confined(&canonical_root, &canonical_path, &path)?;
        let metadata = file
            .metadata()
            .map_err(|_| TransportError::ConnectionFailed)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(TransportError::ConnectionFailed);
        }
        if metadata.len() > self.max_file_size as u64 {
            return Err(TransportError::ResponseTooLarge);
        }

        let body = if request.method == super::HttpMethod::Head {
            Vec::new()
        } else {
            let read_limit = u64::try_from(self.max_file_size)
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            let mut body = Vec::new();
            file.take(read_limit)
                .read_to_end(&mut body)
                .map_err(|_| TransportError::ConnectionFailed)?;
            if body.len() > self.max_file_size {
                return Err(TransportError::ResponseTooLarge);
            }
            body
        };
        let content_length = if request.method == super::HttpMethod::Head {
            metadata.len()
        } else {
            body.len() as u64
        };
        let content_type = content_type_for_path(&path);
        Ok(Response::new(200)
            .with_header("content-type", content_type)
            .with_header("content-length", content_length.to_string())
            .with_body(body))
    }
}

/// Reject pathnames that currently contain a symlink/reparse point in any
/// component. This is defense in depth only: security-sensitive confinement
/// must be enforced by the actual open operation, not just this pathname walk.
fn reject_symlink_path_components(path: &std::path::Path) -> Result<(), TransportError> {
    use std::path::{Component, PathBuf};

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());

        // On Windows, a Prefix component such as `C:` is not itself an
        // absolute filesystem object: checking `C:` before the root separator
        // can be interpreted as a drive-relative path and fail. Check the
        // first real path object only after the root component has been added.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }

        let metadata =
            std::fs::symlink_metadata(&current).map_err(|_| TransportError::ConnectionFailed)?;
        if metadata.file_type().is_symlink() {
            // macOS exposes /var and /tmp as system-managed aliases to
            // /private/var and /private/tmp. Permit only those exact aliases;
            // arbitrary symlinks in user-controlled path components remain denied.
            #[cfg(target_os = "macos")]
            {
                let expected_target = if current == std::path::Path::new("/var") {
                    Some(std::path::Path::new("/private/var"))
                } else if current == std::path::Path::new("/tmp") {
                    Some(std::path::Path::new("/private/tmp"))
                } else {
                    None
                };
                if expected_target.is_some_and(|expected| {
                    std::fs::canonicalize(&current).is_ok_and(|resolved| resolved == expected)
                }) {
                    continue;
                }
                #[cfg(target_os = "macos")]
                eprintln!(
                    "macOS rejected symlink component {:?}, expected target {:?}",
                    current, expected_target
                );
            }
            return Err(TransportError::ConnectionFailed);
        }
    }
    Ok(())
}

/// Open a previously validated target while anchoring resolution to an opened root.
#[cfg(target_os = "linux")]
fn open_local_root_handle_verified(
    canonical_root: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::os::unix::fs::MetadataExt;

    let directory = open_local_root_handle(canonical_root)?;
    let opened_metadata = directory
        .metadata()
        .map_err(|_| TransportError::ConnectionFailed)?;
    let path_metadata =
        std::fs::symlink_metadata(canonical_root).map_err(|_| TransportError::ConnectionFailed)?;

    // The pathname can be replaced between canonicalization and openat2.
    // Verify that the opened directory is the same filesystem object currently
    // named by the granted canonical path; otherwise refuse the grant.
    if !opened_metadata.is_dir()
        || !path_metadata.is_dir()
        || path_metadata.file_type().is_symlink()
        || opened_metadata.dev() != path_metadata.dev()
        || opened_metadata.ino() != path_metadata.ino()
    {
        return Err(TransportError::ConnectionFailed);
    }
    Ok(directory)
}

#[cfg(target_os = "linux")]
fn open_local_root_handle(
    canonical_root: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    const SYS_OPENAT2: isize = 437;
    const AT_FDCWD: i32 = -100;
    const O_CLOEXEC: u64 = 0x80000;
    const O_DIRECTORY: u64 = 0x10000;
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    unsafe extern "C" {
        fn syscall(number: isize, ...) -> isize;
    }

    let root_c = CString::new(canonical_root.as_os_str().as_bytes())
        .map_err(|_| TransportError::ConnectionFailed)?;
    let how = OpenHow {
        flags: O_CLOEXEC | O_DIRECTORY,
        mode: 0,
        resolve: RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    };
    // SAFETY: the path is NUL-terminated and OpenHow matches the openat2 ABI.
    let fd = unsafe {
        syscall(
            SYS_OPENAT2,
            AT_FDCWD,
            root_c.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        return Err(TransportError::ConnectionFailed);
    }
    // SAFETY: successful openat2 returned a new owned file descriptor.
    Ok(unsafe { std::fs::File::from_raw_fd(fd as i32) })
}

#[cfg(all(target_os = "linux", test))]
fn open_local_file_confined(
    canonical_root: &std::path::Path,
    canonical_path: &std::path::Path,
    _original_path: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    let root = open_local_root_handle(canonical_root)?;
    open_local_file_confined_from_handle(&root, canonical_root, canonical_path)
}

#[cfg(target_os = "linux")]
fn open_local_file_confined_from_handle(
    root_file: &std::fs::File,
    canonical_root: &std::path::Path,
    canonical_path: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    const SYS_OPENAT2: isize = 437;
    const O_CLOEXEC: u64 = 0x80000;
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    const RESOLVE_NO_SYMLINKS: u64 = 0x04;
    const RESOLVE_BENEATH: u64 = 0x08;
    unsafe extern "C" {
        fn syscall(number: isize, ...) -> isize;
    }

    let relative = canonical_path
        .strip_prefix(canonical_root)
        .map_err(|_| TransportError::ConnectionFailed)?;
    if relative.as_os_str().is_empty() {
        return Err(TransportError::ConnectionFailed);
    }
    let relative_c = CString::new(relative.as_os_str().as_bytes())
        .map_err(|_| TransportError::ConnectionFailed)?;
    let how = OpenHow {
        flags: O_CLOEXEC,
        mode: 0,
        resolve: RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    };
    // SAFETY: root_file owns a valid directory fd; relative_c and OpenHow remain
    // alive for the syscall, and the kernel copies the provided OpenHow fields.
    let fd = unsafe {
        syscall(
            SYS_OPENAT2,
            root_file.as_raw_fd(),
            relative_c.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        )
    };
    if fd < 0 {
        return Err(TransportError::ConnectionFailed);
    }
    // SAFETY: successful openat2 returned a new owned file descriptor.
    Ok(unsafe { std::fs::File::from_raw_fd(fd as i32) })
}

// macOS and BSD do not all provide Linux openat2. Walk from a pinned directory
// descriptor instead, opening every component relative to the previous handle.
// O_NOFOLLOW applies to each component, not just the final pathname.
#[cfg(all(unix, not(target_os = "linux")))]
fn open_local_root_handle_verified(
    requested_root: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Component;

    // Resolve relative grants against the process working directory, then walk
    // from the filesystem root. Opening only the final absolute pathname with
    // O_NOFOLLOW would still allow an intermediate directory to be swapped for
    // a symlink between the earlier pathname check and this open.
    let mut absolute = if requested_root.is_absolute() {
        requested_root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| TransportError::ConnectionFailed)?
            .join(requested_root)
    };

    // Validate the supplied spelling before resolving macOS's system-managed
    // /var and /tmp aliases. The resolved path is then walked component by
    // component with O_NOFOLLOW, so intermediate components cannot redirect
    // the opened grant after resolution.
    #[cfg(target_os = "macos")]
    {
        reject_symlink_path_components(&absolute).map_err(|error| {
            eprintln!("macOS root spelling rejected {:?}: {:?}", absolute, error);
            error
        })?;
        absolute = std::fs::canonicalize(&absolute).map_err(|error| {
            eprintln!(
                "macOS root canonicalization failed {:?}: {}",
                absolute, error
            );
            TransportError::ConnectionFailed
        })?;
    }

    let mut current = {
        let fd = unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY,
            )
        };
        if fd < 0 {
            return Err(TransportError::ConnectionFailed);
        }
        unsafe { std::fs::File::from_raw_fd(fd) }
    };

    let components: Vec<_> = absolute
        .components()
        .filter_map(|component| match component {
            Component::RootDir | Component::CurDir => None,
            Component::Normal(name) => Some(Ok(name)),
            // Prefixes are not expected on Unix; parent components are rejected
            // rather than normalized across a potentially replaced directory.
            Component::Prefix(_) | Component::ParentDir => {
                Some(Err(TransportError::ConnectionFailed))
            }
        })
        .collect::<Result<_, _>>()?;

    for component in components {
        let name = std::ffi::CString::new(component.as_bytes())
            .map_err(|_| TransportError::ConnectionFailed)?;
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            #[cfg(target_os = "macos")]
            eprintln!(
                "macOS secure root walk failed at {:?}: {}",
                component,
                std::io::Error::last_os_error()
            );
            return Err(TransportError::ConnectionFailed);
        }
        current = unsafe { std::fs::File::from_raw_fd(fd) };
        if !current
            .metadata()
            .map_err(|_| TransportError::ConnectionFailed)?
            .is_dir()
        {
            return Err(TransportError::ConnectionFailed);
        }
    }

    let opened = current
        .metadata()
        .map_err(|_| TransportError::ConnectionFailed)?;
    let named =
        std::fs::symlink_metadata(&absolute).map_err(|_| TransportError::ConnectionFailed)?;
    if !opened.is_dir()
        || !named.is_dir()
        || named.file_type().is_symlink()
        || opened.dev() != named.dev()
        || opened.ino() != named.ino()
    {
        return Err(TransportError::ConnectionFailed);
    }
    Ok(current)
}

#[cfg(all(unix, not(target_os = "linux")))]
fn open_local_file_confined_from_handle(
    root_file: &std::fs::File,
    canonical_root: &std::path::Path,
    canonical_path: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    let relative = canonical_path
        .strip_prefix(canonical_root)
        .map_err(|_| TransportError::ConnectionFailed)?;
    let components: Vec<_> = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value),
            _ => Err(TransportError::ConnectionFailed),
        })
        .collect::<Result<_, _>>()?;
    if components.is_empty() {
        return Err(TransportError::ConnectionFailed);
    }

    // Duplicate the root descriptor so each intermediate directory can be
    // safely replaced by a newly opened descriptor without losing the grant.
    let root_dup = unsafe { libc::fcntl(root_file.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if root_dup < 0 {
        return Err(TransportError::ConnectionFailed);
    }
    let mut current = unsafe { std::fs::File::from_raw_fd(root_dup) };
    for (index, component) in components.iter().enumerate() {
        let name =
            CString::new(component.as_bytes()).map_err(|_| TransportError::ConnectionFailed)?;
        let is_final = index + 1 == components.len();
        let mut flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        if !is_final {
            flags |= libc::O_DIRECTORY;
        }
        // SAFETY: current owns a valid directory descriptor for intermediates;
        // name is NUL-terminated and remains alive throughout openat().
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(TransportError::ConnectionFailed);
        }
        // SAFETY: successful openat() returned a newly owned descriptor.
        let next = unsafe { std::fs::File::from_raw_fd(fd) };
        if !is_final
            && !next
                .metadata()
                .map_err(|_| TransportError::ConnectionFailed)?
                .is_dir()
        {
            return Err(TransportError::ConnectionFailed);
        }
        current = next;
    }
    Ok(current)
}

// Windows: use NtCreateFile with OBJECT_ATTRIBUTES.RootDirectory so each
// component is opened relative to the previously opened directory handle.
// OBJ_DONT_REPARSE and FILE_OPEN_REPARSE_POINT reject reparse-point traversal.
#[cfg(windows)]
fn windows_file_identity(file: &std::fs::File) -> Option<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            handle: Handle,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut information = ByHandleFileInformation::default();
    // SAFETY: the file owns a valid Windows handle and information is writable.
    let success =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    (success != 0 && index != 0).then_some((information.volume_serial_number, index))
}

#[cfg(windows)]
fn open_local_root_handle_verified(
    canonical_root: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    type Handle = *mut std::ffi::c_void;
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation_time: FileTime,
        last_access_time: FileTime,
        last_write_time: FileTime,
        volume_serial_number: u32,
        file_size_high: u32,
        file_size_low: u32,
        number_of_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut std::ffi::c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn GetFileInformationByHandle(
            handle: Handle,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }
    fn file_identity(file: &std::fs::File) -> Option<(u32, u64)> {
        let mut information = ByHandleFileInformation::default();
        // SAFETY: file owns a valid Windows handle and information is writable.
        let success =
            unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
        let index =
            (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
        // A zero file index cannot establish a stable object identity, so fail closed.
        (success != 0 && index != 0).then_some((information.volume_serial_number, index))
    }
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const SHARE_ALL: u32 = 0x7;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    let mut wide: Vec<u16> = canonical_root.as_os_str().encode_wide().collect();
    wide.push(0);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | FILE_READ_ATTRIBUTES,
            SHARE_ALL,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(TransportError::ConnectionFailed);
    }
    let directory = unsafe { std::fs::File::from_raw_handle(handle) };
    let opened = directory
        .metadata()
        .map_err(|_| TransportError::ConnectionFailed)?;
    let named =
        std::fs::symlink_metadata(canonical_root).map_err(|_| TransportError::ConnectionFailed)?;
    // Open the name a second time so identity is compared using the stable
    // Win32 handle API rather than Rust's still-unstable MetadataExt identity APIs.
    let named_file = {
        let mut wide: Vec<u16> = canonical_root.as_os_str().encode_wide().collect();
        wide.push(0);
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | FILE_READ_ATTRIBUTES,
                SHARE_ALL,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(TransportError::ConnectionFailed);
        }
        // SAFETY: CreateFileW returned a new owned handle.
        unsafe { std::fs::File::from_raw_handle(handle) }
    };
    let named_opened = named_file
        .metadata()
        .map_err(|_| TransportError::ConnectionFailed)?;
    let directory_identity = file_identity(&directory);
    let named_identity = file_identity(&named_file);
    if !opened.is_dir()
        || !named.is_dir()
        || opened.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || named.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || named_opened.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || directory_identity.is_none()
        || directory_identity != named_identity
    {
        return Err(TransportError::ConnectionFailed);
    }
    Ok(directory)
}

#[cfg(windows)]
fn open_local_file_confined_from_handle(
    root_file: &std::fs::File,
    canonical_root: &std::path::Path,
    canonical_path: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Component;

    type Handle = *mut std::ffi::c_void;
    type NtStatus = i32;
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root_directory: Handle,
        object_name: *mut UnicodeString,
        attributes: u32,
        security_descriptor: *mut std::ffi::c_void,
        security_quality_of_service: *mut std::ffi::c_void,
    }
    #[repr(C)]
    union IoStatusValue {
        status: NtStatus,
        pointer: *mut std::ffi::c_void,
    }
    #[repr(C)]
    struct IoStatusBlock {
        value: IoStatusValue,
        information: usize,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            file_handle: *mut Handle,
            desired_access: u32,
            object_attributes: *mut ObjectAttributes,
            io_status_block: *mut IoStatusBlock,
            allocation_size: *mut i64,
            file_attributes: u32,
            share_access: u32,
            create_disposition: u32,
            create_options: u32,
            ea_buffer: *mut std::ffi::c_void,
            ea_length: u32,
        ) -> NtStatus;
    }
    unsafe extern "system" {
        fn DuplicateHandle(
            source_process: Handle,
            source: Handle,
            target_process: Handle,
            target: *mut Handle,
            access: u32,
            inherit: i32,
            options: u32,
        ) -> i32;
        fn GetCurrentProcess() -> Handle;
    }
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    const OBJ_DONT_REPARSE: u32 = 0x1000;
    const FILE_OPEN: u32 = 1;
    const FILE_DIRECTORY_FILE: u32 = 0x1;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
    const FILE_READ_DATA: u32 = 0x1;
    const FILE_LIST_DIRECTORY: u32 = 0x1;
    const FILE_READ_ATTRIBUTES: u32 = 0x80;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const SHARE_ALL: u32 = 0x7;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let relative = canonical_path
        .strip_prefix(canonical_root)
        .map_err(|_| TransportError::ConnectionFailed)?;
    let components: Vec<_> = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value),
            _ => Err(TransportError::ConnectionFailed),
        })
        .collect::<Result<_, _>>()?;
    if components.is_empty() {
        return Err(TransportError::ConnectionFailed);
    }

    let process = unsafe { GetCurrentProcess() };
    let mut duplicated: Handle = std::ptr::null_mut();
    if unsafe {
        DuplicateHandle(
            process,
            root_file.as_raw_handle(),
            process,
            &mut duplicated,
            0,
            0,
            0x2,
        )
    } == 0
    {
        return Err(TransportError::ConnectionFailed);
    }
    let mut current = unsafe { std::fs::File::from_raw_handle(duplicated) };
    for (index, component) in components.iter().enumerate() {
        let mut wide: Vec<u16> = component.encode_wide().collect();
        if wide.is_empty() || wide.contains(&0) || wide.len() > u16::MAX as usize / 2 {
            return Err(TransportError::ConnectionFailed);
        }
        let byte_len = (wide.len() * 2) as u16;
        let mut name = UnicodeString {
            length: byte_len,
            maximum_length: byte_len,
            buffer: wide.as_mut_ptr(),
        };
        let mut attributes = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root_directory: current.as_raw_handle(),
            object_name: &mut name,
            attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
            security_descriptor: std::ptr::null_mut(),
            security_quality_of_service: std::ptr::null_mut(),
        };
        let is_final = index + 1 == components.len();
        let access = if is_final {
            FILE_READ_DATA
        } else {
            FILE_LIST_DIRECTORY
        };
        let options = FILE_OPEN_REPARSE_POINT
            | FILE_SYNCHRONOUS_IO_NONALERT
            | if is_final {
                FILE_NON_DIRECTORY_FILE
            } else {
                FILE_DIRECTORY_FILE
            };
        let mut output: Handle = std::ptr::null_mut();
        let mut io = IoStatusBlock {
            value: IoStatusValue { status: 0 },
            information: 0,
        };
        let status = unsafe {
            NtCreateFile(
                &mut output,
                access | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                &mut attributes,
                &mut io,
                std::ptr::null_mut(),
                0,
                SHARE_ALL,
                FILE_OPEN,
                options,
                std::ptr::null_mut(),
                0,
            )
        };
        if status < 0 || output.is_null() {
            return Err(TransportError::ConnectionFailed);
        }
        let next = unsafe { std::fs::File::from_raw_handle(output) };
        let metadata = next
            .metadata()
            .map_err(|_| TransportError::ConnectionFailed)?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (!is_final && !metadata.is_dir())
            || (is_final && !metadata.is_file())
        {
            return Err(TransportError::ConnectionFailed);
        }
        current = next;
    }
    Ok(current)
}

#[cfg(not(any(unix, windows)))]
fn open_local_file_confined(
    _canonical_root: &std::path::Path,
    _canonical_path: &std::path::Path,
    _original_path: &std::path::Path,
) -> Result<std::fs::File, TransportError> {
    Err(TransportError::ConnectionFailed)
}

fn file_url_path(url_path: &str) -> Result<std::path::PathBuf, TransportError> {
    // Keep filesystem path structure stable across URL decoding. The URL parser
    // rejects these forms too, but this boundary must remain safe when called
    // directly or when a future caller bypasses URL canonicalization.
    if url_path.as_bytes().windows(3).any(|escape| {
        escape[0] == b'%'
            && matches!(
                [
                    escape[1].to_ascii_lowercase(),
                    escape[2].to_ascii_lowercase()
                ],
                [b'2', b'f'] | [b'5', b'c']
            )
    }) {
        return Err(TransportError::InvalidRequest);
    }

    let decoded = percent_decode(url_path)?;
    #[cfg(windows)]
    {
        // NTFS alternate data streams use a colon after the drive prefix
        // (for example, C:\\page.html:secret). The URL parser rejects these
        // paths too, but enforce the rule here as well because this function is
        // the filesystem boundary and must remain safe for direct callers.
        let bytes = decoded.as_bytes();
        for (index, byte) in bytes.iter().enumerate() {
            if *byte != b':' {
                continue;
            }
            let drive_colon = if bytes.first() == Some(&b'/') {
                index == 2
                    && bytes.get(1).is_some_and(u8::is_ascii_alphabetic)
                    && bytes.get(3) == Some(&b'/')
            } else {
                index == 1
                    && bytes.first().is_some_and(u8::is_ascii_alphabetic)
                    && bytes.get(2) == Some(&b'/')
            };
            if !drive_colon {
                return Err(TransportError::InvalidRequest);
            }
        }
    }
    if decoded.as_bytes().contains(&0) {
        return Err(TransportError::InvalidUrl(
            super::UrlError::InvalidCharacter,
        ));
    }
    if decoded.contains('\\')
        || decoded
            .split('/')
            .any(|component| matches!(component, "." | ".."))
    {
        return Err(TransportError::InvalidRequest);
    }
    #[cfg(windows)]
    let path = {
        // Do not interpret file URLs as UNC/network-share paths. They can
        // trigger network access and bypass the browser's ordinary HTTP policy.
        if decoded.starts_with("//") {
            return Err(TransportError::InvalidRequest);
        }
        let path = decoded
            .strip_prefix('/')
            .filter(|value| value.as_bytes().get(1) == Some(&b':'))
            .unwrap_or(&decoded);
        // Windows drive-relative paths (for example C:secret.txt) depend on
        // the process's per-drive working directory. Accept only an absolute
        // drive path with a slash after the colon.
        let bytes = path.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'/'
        {
            return Err(TransportError::InvalidRequest);
        }
        path
    };
    #[cfg(not(windows))]
    let path = {
        // A file URL must identify an absolute filesystem path. Resolving a
        // relative path against the browser process working directory would
        // make access depend on launch location and could expose unintended files.
        if !decoded.starts_with('/') {
            return Err(TransportError::InvalidRequest);
        }
        decoded.as_str()
    };
    Ok(std::path::PathBuf::from(path))
}

fn percent_decode(input: &str) -> Result<String, TransportError> {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(TransportError::InvalidRequest);
            }
            let high = hex_value(bytes[index + 1]).ok_or(TransportError::InvalidRequest)?;
            let low = hex_value(bytes[index + 2]).ok_or(TransportError::InvalidRequest)?;
            output.push(high << 4 | low);
            index += 3;
        } else {
            let character = input[index..]
                .chars()
                .next()
                .ok_or(TransportError::InvalidRequest)?;
            let mut encoded = [0_u8; 4];
            let encoded = character.encode_utf8(&mut encoded);
            output.extend_from_slice(encoded.as_bytes());
            index += encoded.len();
        }
    }

    String::from_utf8(output).map_err(|_| TransportError::InvalidRequest)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn content_type_for_path(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("xhtml") => "application/xhtml+xml",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("xml") => "application/xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("csv") => "text/csv; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/vnd.microsoft.icon",
        Some("avif") => "image/avif",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("pdf") => "application/pdf",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

fn validate_request(request: &Request) -> Result<(), TransportError> {
    // CONNECT switches HTTP into tunnel mode and requires authority-form targets.
    // This transport only implements ordinary request/response exchanges; sending
    // CONNECT with the normal origin-form target would be malformed and could
    // accidentally expose tunnel semantics to callers.
    if request.method == super::HttpMethod::Connect {
        return Err(TransportError::InvalidRequest);
    }

    // Bound caller-supplied uploads before DNS lookup or socket creation.
    if request.body.len() > MAX_REQUEST_BODY_BYTES {
        return Err(TransportError::InvalidRequest);
    }
    // GET and HEAD bodies have no interoperable semantics and would make the
    // URL-only HTTP cache key unsafe for requests whose bodies differ.
    if request.has_body()
        && matches!(
            request.method,
            super::HttpMethod::Get | super::HttpMethod::Head
        )
    {
        return Err(TransportError::InvalidRequest);
    }

    let authority = request.url.authority();
    if authority
        .bytes()
        .any(|byte| byte.is_ascii_control() || matches!(byte, b' ' | b'\t'))
        || request
            .url
            .path()
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
        || request.url.query().is_some_and(|query| {
            query
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b' ')
        })
    {
        return Err(TransportError::InvalidRequest);
    }

    if matches!(request.url.scheme(), "http" | "https") {
        validate_http_authority(authority)?;
        if request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .any(|(_, host)| !host.eq_ignore_ascii_case(&host_header(request)))
        {
            return Err(TransportError::InvalidRequest);
        }
    }

    // Bound the headers actually serialized on the wire, including the
    // synthesized Host/Connection/Content-Length fields. The caller-supplied
    // Connection field is replaced with our own "Connection: close" field.
    let has_host_header = request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("host"));
    let has_connection_header = request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("connection"));
    let has_content_length_header = request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("content-length"));
    let serialized_header_count = request
        .headers
        .len()
        .saturating_sub(has_connection_header as usize)
        .saturating_add((!has_host_header) as usize)
        .saturating_add(1)
        .saturating_add((request.has_body() && !has_content_length_header) as usize);
    if serialized_header_count > MAX_REQUEST_HEADER_COUNT {
        return Err(TransportError::InvalidRequest);
    }

    let mut seen_headers = std::collections::HashSet::new();
    for (name, value) in &request.headers {
        if name.is_empty()
            || !name.bytes().all(is_http_token_byte)
            || value.bytes().any(|byte| {
                matches!(byte, b'\r' | b'\n') || byte.is_ascii_control() && byte != b'\t'
            })
            // HTTP field names are case-insensitive. Reject duplicate spellings
            // so framing/security checks cannot inspect a different value than
            // the one a downstream server chooses.
            || !seen_headers.insert(name.to_ascii_lowercase())
        {
            return Err(TransportError::InvalidRequest);
        }
    }

    // This transport connects directly to the origin; it has no proxy hop
    // to authenticate. Forwarding Proxy-Authorization would disclose proxy
    // credentials to the origin server, so reject it rather than serialize it.
    if request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("proxy-authorization"))
    {
        return Err(TransportError::InvalidRequest);
    }

    // Connection-nominated fields are hop-by-hop. Since this transport
    // does not forward a proxy hop, accepting both fields would serialize a
    // header that Connection explicitly says must be removed.
    if let Some(connection) = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("connection"))
        .map(|(_, value)| value.as_str())
    {
        for token in connection.split(',') {
            let token = trim_http_ows(token);
            if token.is_empty() || !token.bytes().all(is_http_token_byte) {
                return Err(TransportError::InvalidRequest);
            }
            if request.headers.keys().any(|name| {
                !name.eq_ignore_ascii_case("connection") && name.eq_ignore_ascii_case(token)
            }) {
                return Err(TransportError::InvalidRequest);
            }
            // Host and Content-Length can be synthesized by write_request()
            // after this validation. A Connection token must not bypass the
            // nominated-field check merely because the caller omitted them.
            if token.eq_ignore_ascii_case("host")
                || (token.eq_ignore_ascii_case("content-length")
                    && request.has_body()
                    && !request
                        .headers
                        .keys()
                        .any(|name| name.eq_ignore_ascii_case("content-length")))
            {
                return Err(TransportError::InvalidRequest);
            }
        }
    }

    // Request.headers is public, so callers can bypass with_header()'s
    // lowercase normalization. Framing checks must therefore be
    // case-insensitive just like HTTP field names, or mixed-case fields can
    // bypass validation while still being serialized onto the wire.
    if request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("transfer-encoding"))
    {
        // This transport writes bodies verbatim and does not implement
        // chunked request encoding.
        return Err(TransportError::InvalidRequest);
    }

    let content_lengths = request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    if content_lengths.len() > 1 {
        return Err(TransportError::InvalidRequest);
    }
    if let Some(length) = content_lengths.first() {
        let length = trim_http_ows(length);
        if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(TransportError::InvalidRequest);
        }
        let length = length
            .parse::<usize>()
            .map_err(|_| TransportError::InvalidRequest)?;
        if length != request.body.len() {
            return Err(TransportError::InvalidRequest);
        }
    }

    // Bound the serialized request line and headers before connecting. Requests
    // are caller-constructible, so a valid but enormous URL/header map must not
    // make the transport allocate or write an unbounded HTTP header block.
    let query_len = request
        .url
        .query()
        .map(|query| {
            query
                .len()
                .checked_add(1)
                .ok_or(TransportError::InvalidRequest)
        })
        .transpose()?
        .unwrap_or(0);
    let target_len = request
        .url
        .path()
        .len()
        .checked_add(query_len)
        .ok_or(TransportError::InvalidRequest)?;
    let mut head_len = request
        .method
        .as_str()
        .len()
        .checked_add(target_len)
        .and_then(|length| length.checked_add(12)) // spaces, HTTP version, CRLF
        .ok_or(TransportError::InvalidRequest)?;
    let has_host = request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("host"));
    if !has_host {
        let host_header_len = host_header(request)
            .len()
            .checked_add(8) // "Host: " + CRLF
            .ok_or(TransportError::InvalidRequest)?;
        head_len = head_len
            .checked_add(host_header_len)
            .ok_or(TransportError::InvalidRequest)?;
    }
    head_len = head_len
        .checked_add(19) // "Connection: close\r\n"
        .ok_or(TransportError::InvalidRequest)?;
    if request.has_body()
        && !request
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    {
        let content_length_header_len = 16usize
            .checked_add(request.body.len().to_string().len())
            .and_then(|length| length.checked_add(2))
            .ok_or(TransportError::InvalidRequest)?;
        head_len = head_len
            .checked_add(content_length_header_len)
            .ok_or(TransportError::InvalidRequest)?;
    }
    for (name, value) in &request.headers {
        if name.eq_ignore_ascii_case("connection") {
            continue;
        }
        head_len = head_len
            .checked_add(name.len())
            .and_then(|length| length.checked_add(value.len()))
            .and_then(|length| length.checked_add(4)) // ": " + CRLF
            .ok_or(TransportError::InvalidRequest)?;
    }
    head_len = head_len
        .checked_add(2) // final CRLF
        .ok_or(TransportError::InvalidRequest)?;
    if head_len > MAX_REQUEST_HEADER_BYTES {
        return Err(TransportError::InvalidRequest);
    }

    Ok(())
}

fn validate_http_authority(authority: &str) -> Result<(), TransportError> {
    // User-info is not used by this browser and makes host parsing ambiguous.
    if authority.is_empty() || authority.contains('@') {
        return Err(TransportError::InvalidRequest);
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let end = bracketed.find(']').ok_or(TransportError::InvalidRequest)?;
        let host = &bracketed[..end];
        host.parse::<std::net::Ipv6Addr>()
            .map_err(|_| TransportError::InvalidRequest)?;
        let suffix = &bracketed[end + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            Some(
                suffix
                    .strip_prefix(':')
                    .ok_or(TransportError::InvalidRequest)?,
            )
        };
        (host, port)
    } else {
        if authority.matches(':').count() > 1 {
            return Err(TransportError::InvalidRequest);
        }
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };

    // A single trailing dot is valid DNS absolute-name notation. Validate
    // the labels without it, while preserving the original authority for the
    // request target and Host header.
    let hostname = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty()
        || (!authority.starts_with('[')
            && (hostname.is_empty()
                || hostname.starts_with('.')
                || hostname.split('.').any(|label| {
                    label.is_empty()
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })))
    {
        return Err(TransportError::InvalidRequest);
    }

    if let Some(port) = port {
        let parsed = port
            .parse::<u16>()
            .map_err(|_| TransportError::InvalidRequest)?;
        if parsed == 0 {
            return Err(TransportError::InvalidRequest);
        }
    }

    Ok(())
}

fn socket_address(url: &super::Url) -> String {
    let host = url.host();
    if host.contains(':') {
        format!("[{host}]:{}", url.effective_port())
    } else {
        format!("{host}:{}", url.effective_port())
    }
}

fn host_header(request: &Request) -> String {
    let host = request.url.host();
    let default_port = matches!(
        (request.url.scheme(), request.url.effective_port()),
        ("http", 80) | ("https", 443)
    );
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    if default_port {
        host
    } else {
        format!("{host}:{}", request.url.effective_port())
    }
}

/// Parse one complete HTTP/1.1 response using caller-supplied byte limits.
///
/// This entry point is useful for offline validation of untrusted response bytes;
/// it performs no network I/O.
pub fn parse_http_response(
    bytes: &[u8],
    max_response_size: usize,
    max_header_size: usize,
) -> Result<Response, TransportError> {
    parse_http_response_for_method(
        bytes,
        super::HttpMethod::Get,
        max_response_size,
        max_header_size,
    )
}

fn parse_http_response_for_method(
    bytes: &[u8],
    method: super::HttpMethod,
    max_response_size: usize,
    max_header_size: usize,
) -> Result<Response, TransportError> {
    let mut cursor = 0;
    let mut interim_responses = 0;

    loop {
        let (response, body_start) = parse_http_response_head(&bytes[cursor..], max_header_size)?;

        // Protocol upgrades are unsupported; accepting 101 as a final response
        // would expose a connection state this HTTP/1.1 client cannot handle.
        if response.status == 101 {
            return Err(TransportError::ConnectionFailed);
        }

        if is_interim_response(response.status) {
            // Informational responses cannot carry message framing fields.
            // Reject them rather than letting a peer disagree about where the
            // next response starts.
            if response.header("content-length").is_some()
                || response.header("transfer-encoding").is_some()
            {
                return Err(TransportError::ConnectionFailed);
            }
            interim_responses += 1;
            if interim_responses > MAX_INTERIM_RESPONSES {
                return Err(TransportError::ConnectionFailed);
            }
            cursor = cursor
                .checked_add(body_start)
                .ok_or(TransportError::ConnectionFailed)?;
            if cursor >= bytes.len() {
                return Err(TransportError::ConnectionFailed);
            }
            continue;
        }

        let body_offset = cursor
            .checked_add(body_start)
            .ok_or(TransportError::ConnectionFailed)?;
        let body_bytes = bytes
            .get(body_offset..)
            .ok_or(TransportError::ConnectionFailed)?;
        let transfer_encoding = response.header("transfer-encoding");
        let content_length = response.header("content-length");
        if response.status == 204 && (transfer_encoding.is_some() || content_length.is_some()) {
            return Err(TransportError::ConnectionFailed);
        }
        if transfer_encoding.is_some() && content_length.is_some() {
            return Err(TransportError::ConnectionFailed);
        }
        // Validate Content-Length even on HEAD and body-forbidden status codes.
        // Those responses carry no body, but malformed framing metadata must
        // not become accepted merely because the body branch is skipped.
        let parsed_content_length = content_length
            .map(|length| {
                let length = trim_http_ows(length);
                if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(TransportError::ConnectionFailed);
                }
                length
                    .parse::<usize>()
                    .map_err(|_| TransportError::ConnectionFailed)
            })
            .transpose()?;
        // 205 Reset Content must carry no content. Reject transfer coding
        // (which this no-body parser cannot consume) and non-zero lengths.
        if response.status == 205
            && (transfer_encoding.is_some()
                || parsed_content_length.is_some_and(|length| length != 0))
        {
            return Err(TransportError::ConnectionFailed);
        }
        let is_chunked = match transfer_encoding {
            Some(value) if trim_http_ows(value).eq_ignore_ascii_case("chunked") => true,
            Some(_) => return Err(TransportError::ConnectionFailed),
            None => false,
        };

        let body_forbidden = matches!(method, crate::net::HttpMethod::Head)
            || matches!(response.status, 204 | 205 | 304);

        let body = if body_forbidden {
            if !body_bytes.is_empty() {
                return Err(TransportError::ConnectionFailed);
            }
            Vec::new()
        } else if is_chunked {
            decode_chunked(body_bytes, max_response_size, max_header_size)?
        } else if let Some(length) = parsed_content_length {
            if length > max_response_size {
                return Err(TransportError::ResponseTooLarge);
            }
            if body_bytes.len() != length {
                return Err(TransportError::ConnectionFailed);
            }
            body_bytes.to_vec()
        } else {
            body_bytes.to_vec()
        };

        if body.len() > max_response_size {
            return Err(TransportError::ResponseTooLarge);
        }

        return Ok(response.with_body(body));
    }
}

fn parse_http_response_head(
    bytes: &[u8],
    max_header_size: usize,
) -> Result<(Response, usize), TransportError> {
    let separator = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(TransportError::ConnectionFailed)?;
    let header_bytes = &bytes[..separator];
    if header_bytes.len() > max_header_size {
        return Err(TransportError::ResponseTooLarge);
    }

    let header_text =
        std::str::from_utf8(header_bytes).map_err(|_| TransportError::ConnectionFailed)?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or(TransportError::ConnectionFailed)?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next().unwrap_or_default();
    let status_text = status_parts.next().unwrap_or_default();
    let Some(reason) = status_parts.next() else {
        return Err(TransportError::ConnectionFailed);
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || status_text.len() != 3
        || !status_text.bytes().all(|byte| byte.is_ascii_digit())
        || reason
            .bytes()
            .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
    {
        return Err(TransportError::ConnectionFailed);
    }
    let status = status_text
        .parse::<u16>()
        .map_err(|_| TransportError::ConnectionFailed)?;
    if !(100..=599).contains(&status) || status == 101 {
        // Protocol upgrades are not supported by this request/response transport.
        // Do not expose upgraded protocol bytes to the document parser.
        return Err(TransportError::ConnectionFailed);
    }

    let mut response = Response::new(status);
    let mut seen_headers = std::collections::HashSet::new();
    let mut header_count = 0;
    for line in lines {
        header_count += 1;
        if header_count > MAX_RESPONSE_HEADER_COUNT {
            return Err(TransportError::ResponseTooLarge);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(TransportError::ConnectionFailed);
        };
        // Field names must be HTTP tokens; trimming malformed names can turn
        // an invalid response into a different, accepted header. Reject
        // duplicates case-insensitively so consumers cannot observe a
        // merged/overwritten value different from the validated sequence.
        if name.is_empty()
            || !name.bytes().all(is_http_token_byte)
            || value
                .bytes()
                .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
            || (!seen_headers.insert(name.to_ascii_lowercase())
                && !name.eq_ignore_ascii_case("set-cookie"))
        {
            return Err(TransportError::ConnectionFailed);
        }
        let value = trim_http_ows(value);
        response = response.with_header(name, value);
    }

    // A Connection field can nominate otherwise ordinary headers as
    // hop-by-hop. If we expose those fields to MIME, cookie, or cache policy,
    // different layers may interpret a response differently. This client does
    // not forward hop-by-hop metadata, so reject a response that nominates any
    // other field it actually carries. Validate the token list as well.
    if let Some(connection) = response.header("connection") {
        for token in connection.split(',') {
            let token = trim_http_ows(token);
            if token.is_empty() || !token.bytes().all(is_http_token_byte) {
                return Err(TransportError::ConnectionFailed);
            }
            if response.headers.keys().any(|name| {
                !name.eq_ignore_ascii_case("connection") && name.eq_ignore_ascii_case(token)
            }) {
                return Err(TransportError::ConnectionFailed);
            }
        }
    }

    // Transfer-Encoding is an HTTP/1.1 framing mechanism. Accepting it on an
    // HTTP/1.0 response risks disagreement with peers that ignore the field.
    if version == "HTTP/1.0" && response.header("transfer-encoding").is_some() {
        return Err(TransportError::ConnectionFailed);
    }

    Ok((response, separator + 4))
}

fn trim_http_ows(value: &str) -> &str {
    // HTTP optional whitespace is ASCII SP / HTAB only. Unicode str::trim()
    // accepts characters such as NBSP that remain distinct bytes on the wire,
    // creating parser/peer disagreement around framing fields.
    value.trim_matches(|character| character == ' ' || character == '\t')
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            33 | 35..=39 | 42..=43 | 45..=46 | 94..=96 | 124 | 126
        )
}

fn is_interim_response(status: u16) -> bool {
    (100..200).contains(&status) && status != 101
}

fn valid_chunk_extensions(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b';') {
            return false;
        }
        cursor += 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }

        let name_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| is_http_token_byte(*byte))
        {
            cursor += 1;
        }
        if cursor == name_start {
            return false;
        }

        while bytes
            .get(cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            cursor += 1;
        }
        if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while bytes
                .get(cursor)
                .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            {
                cursor += 1;
            }

            if bytes.get(cursor) == Some(&b'"') {
                cursor += 1;
                let mut closed = false;
                while let Some(byte) = bytes.get(cursor).copied() {
                    cursor += 1;
                    match byte {
                        b'"' => {
                            closed = true;
                            break;
                        }
                        b'\\' => {
                            let Some(escaped) = bytes.get(cursor).copied() else {
                                return false;
                            };
                            if !(escaped == b'\t' || (b' '..=b'~').contains(&escaped)) {
                                return false;
                            }
                            cursor += 1;
                        }
                        b'\t' | b' ' | b'!' | b'#'..=b'[' | b']'..=b'~' => {}
                        _ => return false,
                    }
                }
                if !closed {
                    return false;
                }
            } else {
                let value_start = cursor;
                while bytes
                    .get(cursor)
                    .is_some_and(|byte| is_http_token_byte(*byte))
                {
                    cursor += 1;
                }
                if cursor == value_start {
                    return false;
                }
            }
        }

        if cursor == bytes.len() {
            return true;
        }
        if !bytes[cursor..]
            .iter()
            .any(|byte| !matches!(byte, b' ' | b'\t'))
        {
            return true;
        }
    }

    false
}

fn decode_chunked(
    bytes: &[u8],
    max_response_size: usize,
    max_header_size: usize,
) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::new();
    let mut cursor = 0;

    loop {
        let relative_end = bytes[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(TransportError::ConnectionFailed)?;
        if relative_end > max_header_size {
            return Err(TransportError::ResponseTooLarge);
        }
        let line_end = cursor
            .checked_add(relative_end)
            .ok_or(TransportError::ConnectionFailed)?;
        let line = std::str::from_utf8(&bytes[cursor..line_end])
            .map_err(|_| TransportError::ConnectionFailed)?;
        if line
            .bytes()
            .any(|byte| !byte.is_ascii() || (byte < 0x20 && byte != b'\t') || byte == 0x7f)
        {
            return Err(TransportError::ConnectionFailed);
        }
        let (size_text, extensions) = match line.split_once(';') {
            Some((size, extensions)) => (size.trim_end_matches([' ', '\t']), Some(extensions)),
            None => (line, None),
        };
        if size_text.is_empty()
            || !size_text.bytes().all(|byte| byte.is_ascii_hexdigit())
            || extensions.is_some_and(|extensions| {
                let extensions = format!(";{extensions}");
                !valid_chunk_extensions(&extensions)
            })
        {
            return Err(TransportError::ConnectionFailed);
        }
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| TransportError::ConnectionFailed)?;
        cursor = line_end
            .checked_add(2)
            .ok_or(TransportError::ConnectionFailed)?;

        if size == 0 {
            let trailer_bytes = &bytes[cursor..];
            if trailer_bytes == b"\r\n" {
                return Ok(output);
            }
            let trailer_end = trailer_bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .ok_or(TransportError::ConnectionFailed)?;
            if trailer_end > max_header_size {
                return Err(TransportError::ResponseTooLarge);
            }
            if trailer_end
                .checked_add(4)
                .ok_or(TransportError::ConnectionFailed)?
                != trailer_bytes.len()
            {
                return Err(TransportError::ConnectionFailed);
            }

            let trailer_text = std::str::from_utf8(&trailer_bytes[..trailer_end])
                .map_err(|_| TransportError::ConnectionFailed)?;
            let mut seen_trailers = std::collections::HashSet::new();
            let mut trailer_count = 0;
            for line in trailer_text.split("\r\n") {
                trailer_count += 1;
                if trailer_count > MAX_RESPONSE_HEADER_COUNT {
                    return Err(TransportError::ResponseTooLarge);
                }
                let Some((name, value)) = line.split_once(':') else {
                    return Err(TransportError::ConnectionFailed);
                };
                if name.is_empty()
                    || !name.bytes().all(is_http_token_byte)
                    || value
                        .bytes()
                        .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
                    || !seen_trailers.insert(name.to_ascii_lowercase())
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        // Trailer fields must not override message framing,
                        // routing, authentication, or metadata that changes
                        // how the already-decoded representation is handled.
                        "content-length"
                            | "transfer-encoding"
                            | "host"
                            | "trailer"
                            | "authorization"
                            | "proxy-authorization"
                            | "cache-control"
                            | "content-encoding"
                            | "content-type"
                            | "content-range"
                            | "content-language"
                            | "content-location"
                            | "content-disposition"
                            | "content-md5"
                            | "digest"
                            | "last-modified"
                            | "set-cookie"
                            | "www-authenticate"
                            | "proxy-authenticate"
                            | "strict-transport-security"
                            | "max-forwards"
                            | "te"
                            | "expires"
                            | "location"
                            | "retry-after"
                            | "vary"
                            | "connection"
                            | "keep-alive"
                            | "proxy-connection"
                            | "upgrade"
                            | "expect"
                            | "authentication-info"
                            | "proxy-authentication-info"
                            | "cookie"
                    )
                {
                    return Err(TransportError::ConnectionFailed);
                }
            }
            return Ok(output);
        }

        let end = cursor
            .checked_add(size)
            .ok_or(TransportError::ConnectionFailed)?;
        if output.len().saturating_add(size) > max_response_size {
            return Err(TransportError::ResponseTooLarge);
        }
        output.extend_from_slice(
            bytes
                .get(cursor..end)
                .ok_or(TransportError::ConnectionFailed)?,
        );
        let terminator_end = end.checked_add(2).ok_or(TransportError::ConnectionFailed)?;
        if bytes.get(end..terminator_end) != Some(b"\r\n") {
            return Err(TransportError::ConnectionFailed);
        }
        cursor = terminator_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Url;

    // URL paths always use forward slashes, including Windows drive paths.
    // Path::display() alone emits backslashes on Windows, which the URL parser
    // correctly rejects as ambiguous separators.
    fn local_file_test_url(path: &std::path::Path) -> String {
        let path = path.to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            format!("file:///{}", path)
        } else {
            format!("file://{}", path)
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_temp_alias_is_supported_by_secure_root_walk() {
        let root =
            std::env::temp_dir().join(format!("archia-browser-macos-alias-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();

        let transport = LocalFileTransport::new().with_allowed_roots([root.clone()]);
        assert_eq!(
            transport.authorized_root_handles.len(),
            1,
            "the OS-managed /var -> /private/var alias must not prevent granting a temp directory"
        );

        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn detects_common_local_web_asset_content_types_case_insensitively() {
        for (path, expected) in [
            ("index.HTML", "text/html; charset=utf-8"),
            ("app.js", "text/javascript; charset=utf-8"),
            ("app.mjs", "text/javascript; charset=utf-8"),
            ("data.json", "application/json"),
            ("site.CSS", "text/css; charset=utf-8"),
            ("image.PNG", "image/png"),
            ("font.woff2", "font/woff2"),
            ("module.wasm", "application/wasm"),
            ("unknown.custom", "application/octet-stream"),
        ] {
            assert_eq!(
                content_type_for_path(std::path::Path::new(path)),
                expected,
                "unexpected MIME type for {path}"
            );
        }
    }

    fn tls_fixture_section(name: &str) -> Vec<u8> {
        let fixtures =
            include_str!("../../tests/fixtures/tls-validation.pem").replace("\r\n", "\n");
        let marker = format!("### {name}\n");
        let section = fixtures.split_once(&marker).expect("TLS fixture section").1;
        let pem = section.split("\n### ").next().unwrap_or(section);
        let encoded: String = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .flat_map(str::chars)
            .collect();
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut output = Vec::new();
        let mut accumulator = 0_u32;
        let mut bits = 0_u8;
        for byte in encoded.bytes().filter(|byte| *byte != b'=') {
            let value = alphabet
                .iter()
                .position(|candidate| *candidate == byte)
                .expect("valid base64 fixture") as u32;
            accumulator = (accumulator << 6) | value;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                output.push((accumulator >> bits) as u8);
                accumulator &= (1_u32 << bits).wrapping_sub(1);
            }
        }
        output
    }

    fn tls_fixture_handshake(
        certificate_label: &str,
        client_name: &str,
        trust_test_ca: bool,
    ) -> bool {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        use rustls::{ServerConfig, ServerConnection};
        use std::net::TcpListener;
        use std::thread;

        let certificate = CertificateDer::from(tls_fixture_section(certificate_label));
        let private_key =
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(tls_fixture_section("SERVER_KEY")));
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)
            .expect("test certificate and key must form a server identity");
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut connection = ServerConnection::new(Arc::new(server_config)).unwrap();
            while connection.is_handshaking() {
                if connection.complete_io(&mut stream).is_err() {
                    break;
                }
            }
        });

        let mut roots = RootCertStore::empty();
        if trust_test_ca {
            roots
                .add(CertificateDer::from(tls_fixture_section("CA_CERT")))
                .expect("test CA must be parseable");
        }
        let client_config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let result = (|| {
            let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).ok()?;
            stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
            stream
                .set_write_timeout(Some(Duration::from_secs(3)))
                .ok()?;
            let name = ServerName::try_from(client_name.to_owned()).ok()?;
            let mut connection = ClientConnection::new(Arc::new(client_config), name).ok()?;
            while connection.is_handshaking() {
                connection.complete_io(&mut stream).ok()?;
            }
            Some(())
        })()
        .is_some();
        let _ = server.join();
        result
    }

    #[test]
    fn tls_handshake_validates_trust_chain_expiry_and_hostname() {
        assert!(
            tls_fixture_handshake("VALID_CERT", "localhost", true),
            "a currently valid certificate for the requested host, signed by a trusted CA, must succeed"
        );
        assert!(
            !tls_fixture_handshake("EXPIRED_CERT", "localhost", true),
            "an expired certificate must be rejected even when its issuer is trusted"
        );
        assert!(
            !tls_fixture_handshake("WRONG_HOST_CERT", "localhost", true),
            "a trusted certificate for a different hostname must be rejected"
        );
        assert!(
            !tls_fixture_handshake("VALID_CERT", "localhost", false),
            "a valid certificate from an untrusted CA must be rejected"
        );
    }

    #[test]
    fn tls_client_advertises_only_http_11() {
        let config = tls_client_config();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
    }

    #[test]
    fn tls_handshake_rejects_a_non_tls_peer_as_a_tls_failure() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // A peer that sends plaintext instead of a TLS handshake must not
            // be mistaken for an HTTP response or accepted as a secure origin.
            let _ = stream.write_all(b"plaintext response");
        });

        let url = Url::parse(&format!("https://127.0.0.1:{}/", address.port())).unwrap();
        let transport =
            HttpTransport::with_timeouts(Duration::from_secs(2), Duration::from_secs(2));
        assert_eq!(
            transport.send(&Request::new(url)),
            Err(TransportError::TlsFailed),
            "plaintext from a TLS endpoint must fail closed during the handshake"
        );
        server.join().unwrap();
    }

    #[test]
    fn creates_tls_server_names_for_dns_and_ip_hosts() {
        assert!(tls_server_name("example.org").is_ok());
        assert!(tls_server_name("127.0.0.1").is_ok());
        assert!(tls_server_name("::1").is_ok());
        assert!(tls_server_name("").is_err());
        assert!(tls_server_name("bad host.example").is_err());
        assert!(tls_server_name("*.example.org").is_err());
        assert!(tls_server_name("example.org/path").is_err());
    }

    #[test]
    fn rejects_connection_nominated_representation_and_framing_headers() {
        for response in [
            b"HTTP/1.1 200 OK\r\nConnection: Content-Type\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nConnection: Content-Length\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nConnection: Set-Cookie\r\nSet-Cookie: sid=secret\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nConnection: ,close\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nConnection: bad token\r\nContent-Length: 0\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn accepts_connection_close_without_nominated_headers() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nOK",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(response.body, b"OK");
    }

    #[test]
    fn rejects_unsupported_protocol_switching_responses() {
        assert_eq!(
            parse_http_response(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
                1024,
                1024,
            ),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn parses_content_length_response() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 5\r\n\r\nHello",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(
            response.content_type.as_deref(),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn decodes_chunked_response_with_trailer() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\nX-Test: yes\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn rejects_bytes_after_chunked_trailers() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nunexpected";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_invalid_chunk_size_lines() {
        for chunk_size in [
            " 5",
            " 5;name",
            "5 ",
            "+5",
            "5;bad\u{1}extension",
            "5;",
            "5;=value",
            "5;name=",
            r#"5;name="unterminated"#,
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{chunk_size}\r\nHello\r\n0\r\n\r\n"
            );
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "chunk size line must be rejected: {chunk_size:?}"
            );
        }
    }

    #[test]
    fn accepts_valid_chunk_extensions() {
        for extension in [
            "name",
            "name=value",
            "name=\"quoted value\"",
            "name = token",
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;{extension}\r\nHello\r\n0\r\n\r\n"
            );
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024)
                    .unwrap()
                    .body,
                b"Hello"
            );
        }
    }

    #[test]
    fn rejects_chunked_trailers_that_change_representation_metadata() {
        for trailer in [
            "Content-Type: text/html",
            "Content-Encoding: gzip",
            "Content-Range: bytes 0-4/5",
            "Content-Language: en",
            "Content-Location: /other",
            "Content-Disposition: attachment",
            "Content-MD5: d41d8cd98f00b204e9800998ecf8427e",
            "Digest: sha-256=abc",
            "Last-Modified: Wed, 21 Oct 2015 07:28:00 GMT",
            "Set-Cookie: sid=secret",
            "WWW-Authenticate: Basic realm=private",
            "Proxy-Authenticate: Basic",
            "Strict-Transport-Security: max-age=31536000",
            "Cache-Control: no-store",
            "Location: https://attacker.example/",
            "Authorization: Basic dGVzdA==",
            "Trailer: X-Other",
            "Connection: keep-alive",
            "Keep-Alive: timeout=5",
            "Proxy-Connection: keep-alive",
            "Upgrade: websocket",
            "Expect: 100-continue",
            "Authentication-Info: nextnonce=abc",
            "Proxy-Authentication-Info: nextnonce=abc",
            "Proxy-Authorization: Basic dGVzdA==",
            "Cookie: sid=secret",
            "Host: attacker.example",
            "TE: trailers",
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n{trailer}\r\n\r\n"
            );
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "representation-changing trailer must be rejected: {trailer}"
            );
        }
    }

    #[test]
    fn rejects_malformed_or_framing_chunked_trailers() {
        for trailer in [
            "Bad Header: value",
            "Content-Length: 0",
            "Transfer-Encoding: chunked",
            "X-Test: bad\u{1}value",
            "X-Test: one\r\nx-test: two",
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n{trailer}\r\n\r\n"
            );
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "trailer must be rejected: {trailer}"
            );
        }
    }

    #[test]
    fn decodes_chunked_response() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn body_forbidden_responses_still_validate_content_length_syntax() {
        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: +5\r\n\r\nHello".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: -0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: \r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }

        for (response, method) in [
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\n\r\n".as_slice(),
                crate::net::HttpMethod::Head,
            ),
            (
                b"HTTP/1.1 204 No Content\r\nContent-Length: invalid\r\n\r\n".as_slice(),
                crate::net::HttpMethod::Get,
            ),
            (
                b"HTTP/1.1 304 Not Modified\r\nContent-Length: \xC2\xA05\xC2\xA0\r\n\r\n"
                    .as_slice(),
                crate::net::HttpMethod::Get,
            ),
        ] {
            assert_eq!(
                parse_http_response_for_method(response, method, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn head_response_ignores_declared_body_length() {
        let response = parse_http_response_for_method(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n",
            crate::net::HttpMethod::Head,
            1024,
            1024,
        )
        .unwrap();
        assert!(response.body.is_empty());
    }

    #[test]
    fn no_content_response_has_no_body() {
        let response = parse_http_response(b"HTTP/1.1 204 No Content\r\n\r\n", 1024, 1024).unwrap();
        assert!(response.body.is_empty());
    }

    #[test]
    fn rejects_unsupported_and_ambiguous_transfer_encodings() {
        for encoding in ["gzip", "gzip, chunked", "chunked, gzip"] {
            let response =
                format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: {encoding}\r\n\r\n0\r\n\r\n");
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "encoding {encoding} must not be partially decoded"
            );
        }
    }

    #[test]
    fn rejects_extra_bytes_after_content_length_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHelloextra";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn reset_content_response_must_not_contain_body_bytes() {
        let empty = parse_http_response(b"HTTP/1.1 205 Reset Content\r\n\r\n", 1024, 1024);
        assert_eq!(empty.unwrap().body, b"");

        let with_body =
            parse_http_response(b"HTTP/1.1 205 Reset Content\r\n\r\nunexpected", 1024, 1024);
        assert_eq!(with_body, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_body_bytes_for_head_and_no_content_responses() {
        let head = parse_http_response_for_method(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\nunexpected",
            crate::net::HttpMethod::Head,
            1024,
            1024,
        );
        assert_eq!(head, Err(TransportError::ConnectionFailed));

        let no_content =
            parse_http_response(b"HTTP/1.1 204 No Content\r\n\r\nunexpected", 1024, 1024);
        assert_eq!(no_content, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_duplicate_response_headers_case_insensitively() {
        for response in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nHello".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\ncontent-type: text/html\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn preserves_multiple_set_cookie_headers() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nset-cookie: b=2\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(
            response.set_cookie_headers(),
            &["a=1".to_string(), "b=2".to_string()]
        );
        assert_eq!(response.header("set-cookie"), Some("b=2"));
    }

    #[test]
    fn skips_multiple_informational_responses_before_final_response() {
        let response = parse_http_response(
            b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn reset_content_requires_zero_length_framing() {
        for response in [
            b"HTTP/1.1 205 Reset Content\r\nContent-Length: 1\r\n\r\n".as_slice(),
            b"HTTP/1.1 205 Reset Content\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }

        let zero_length = parse_http_response(
            b"HTTP/1.1 205 Reset Content\r\nContent-Length: 0\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();
        assert!(zero_length.body.is_empty());
    }

    #[test]
    fn rejects_framing_headers_on_informational_and_204_responses() {
        for response in [
            b"HTTP/1.1 100 Continue\r\nContent-Length: 0\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 103 Early Hints\r\nTransfer-Encoding: chunked\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 204 No Content\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn rejects_excessive_informational_responses() {
        let mut bytes = Vec::new();
        for _ in 0..=MAX_INTERIM_RESPONSES {
            bytes.extend_from_slice(b"HTTP/1.1 103 Early Hints\r\n\r\n");
        }
        bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");

        assert_eq!(
            parse_http_response(&bytes, 4096, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_response_with_too_many_headers_or_trailers() {
        let mut headers = String::from("HTTP/1.1 200 OK\r\n");
        for index in 0..=MAX_RESPONSE_HEADER_COUNT {
            headers.push_str(&format!("X-Test-{index}: value\r\n"));
        }
        headers.push_str("\r\n");
        assert_eq!(
            parse_http_response(headers.as_bytes(), 4096, 64 * 1024),
            Err(TransportError::ResponseTooLarge)
        );

        let mut trailers =
            String::from("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n");
        for index in 0..=MAX_RESPONSE_HEADER_COUNT {
            trailers.push_str(&format!("X-Test-{index}: value\r\n"));
        }
        trailers.push_str("\r\n");
        assert_eq!(
            parse_http_response(trailers.as_bytes(), 4096, 64 * 1024),
            Err(TransportError::ResponseTooLarge)
        );
    }

    #[test]
    fn rejects_invalid_response_header_names_and_control_bytes() {
        for response in [
            b"HTTP/1.1 200 OK\r\nBad Header: value\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nX-Test: safe\x01value\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\n: missing-name\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn rejects_invalid_http_versions_and_status_codes() {
        for status_line in [
            "HTTP/9.9 200 OK",
            "HTTP/1.1 20 OK",
            "HTTP/1.1 200",
            "HTTP/1.1 600 Unknown",
            "HTTP/1.1 200 Bad\u{1}Reason",
        ] {
            let response = format!("{status_line}\r\n\r\n");
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "status line must be rejected: {status_line:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_status() {
        assert!(parse_http_response(b"not-http\r\n\r\nbody", 1024, 1024).is_err());
    }

    #[test]
    fn request_body_limit_accepts_n_minus_one_and_exact_boundary_and_rejects_one_over() {
        let url = Url::parse("https://example.org/upload").unwrap();
        let under = Request::new(url.clone())
            .with_method(super::super::HttpMethod::Post)
            .with_body(vec![b'x'; MAX_REQUEST_BODY_BYTES - 1]);
        assert_eq!(
            validate_request(&under),
            Ok(()),
            "N-1 request body bytes must fit"
        );

        let exact = Request::new(url.clone())
            .with_method(super::super::HttpMethod::Post)
            .with_body(vec![b'x'; MAX_REQUEST_BODY_BYTES]);
        assert_eq!(
            validate_request(&exact),
            Ok(()),
            "N request body bytes must fit"
        );

        let oversized = Request::new(url)
            .with_method(super::super::HttpMethod::Post)
            .with_body(vec![b'x'; MAX_REQUEST_BODY_BYTES + 1]);
        assert_eq!(
            validate_request(&oversized),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_bodies_on_get_and_head_requests() {
        let url = Url::parse("https://example.org/resource").unwrap();
        for method in [crate::net::HttpMethod::Get, crate::net::HttpMethod::Head] {
            let request = Request::new(url.clone())
                .with_method(method)
                .with_body(b"ambiguous request body".to_vec());
            assert_eq!(
                validate_request(&request),
                Err(TransportError::InvalidRequest),
                "{method:?} requests must not carry a body"
            );
        }
        assert_eq!(
            validate_request(&Request::new(url)),
            Ok(()),
            "bodyless GET remains valid"
        );
    }

    #[test]
    fn request_header_count_limit_covers_n_minus_one_n_and_n_plus_one() {
        let url = Url::parse("https://example.org/").unwrap();
        // With no caller-supplied Host or Connection, the transport adds
        // those two fields itself. Count the actual serialized header block.
        let mut under = Request::new(url.clone());
        for index in 0..MAX_REQUEST_HEADER_COUNT - 3 {
            under
                .headers
                .insert(format!("x-test-{index}"), "v".to_owned());
        }
        assert_eq!(
            validate_request(&under),
            Ok(()),
            "N-1 serialized request headers must fit"
        );

        let mut exact = Request::new(url.clone());
        for index in 0..MAX_REQUEST_HEADER_COUNT - 2 {
            exact
                .headers
                .insert(format!("x-test-{index}"), "v".to_owned());
        }
        assert_eq!(
            validate_request(&exact),
            Ok(()),
            "N serialized request headers must fit"
        );

        let mut oversized = Request::new(url);
        for index in 0..=MAX_REQUEST_HEADER_COUNT - 2 {
            oversized
                .headers
                .insert(format!("x-test-{index}"), "v".to_owned());
        }
        assert_eq!(
            validate_request(&oversized),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_mismatched_mixed_case_host_before_opening_a_socket() {
        use std::io::ErrorKind;
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/", address.port())).unwrap();
        let mut request = Request::new(url);
        // Public headers can bypass builder normalization, so validation must
        // compare Host case-insensitively before the transport connects.
        request
            .headers
            .insert("hOsT".to_owned(), "attacker.example".to_owned());

        assert_eq!(
            HttpTransport::new().send(&request),
            Err(TransportError::InvalidRequest)
        );
        assert!(
            matches!(
                listener.accept(),
                Err(ref error) if error.kind() == ErrorKind::WouldBlock
            ),
            "an invalid Host header must be rejected before a TCP connection is opened"
        );
    }

    #[test]
    fn rejects_injected_and_case_duplicate_headers_before_opening_a_socket() {
        use std::io::ErrorKind;
        use std::net::TcpListener;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/", address.port())).unwrap();

        let mut injected = Request::new(url.clone());
        injected
            .headers
            .insert("x-test".to_owned(), "safe\r\nX-Injected: yes".to_owned());

        let mut duplicate = Request::new(url.clone());
        duplicate
            .headers
            .insert("x-test".to_owned(), "one".to_owned());
        duplicate
            .headers
            .insert("X-Test".to_owned(), "two".to_owned());

        let mut invalid_name = Request::new(url);
        invalid_name
            .headers
            .insert("bad name".to_owned(), "value".to_owned());

        for request in [injected, duplicate, invalid_name] {
            assert_eq!(
                HttpTransport::new().send(&request),
                Err(TransportError::InvalidRequest)
            );
            assert!(
                matches!(
                    listener.accept(),
                    Err(ref error) if error.kind() == ErrorKind::WouldBlock
                ),
                "invalid headers must be rejected before a TCP connection is opened"
            );
        }
    }

    #[test]
    fn rejects_non_http_schemes_before_connecting() {
        let transport = HttpTransport::new();
        let request = Request::new(Url::parse("ftp://example.org/file").unwrap());
        assert_eq!(
            transport.send(&request),
            Err(TransportError::UnsupportedScheme)
        );
    }

    #[test]
    fn percent_decode_preserves_utf8_paths() {
        assert_eq!(percent_decode("/tmp/%C3%A9.html").unwrap(), "/tmp/é.html");
        assert_eq!(
            percent_decode("/tmp/caf%C3%A9.html").unwrap(),
            "/tmp/café.html"
        );
    }

    #[test]
    fn unavailable_allowed_root_does_not_disable_other_valid_roots() {
        let base = std::env::temp_dir().join(format!(
            "archia-browser-multiple-roots-{}",
            std::process::id()
        ));
        let valid_root = base.join("valid");
        let missing_root = base.join("missing");
        std::fs::create_dir_all(&valid_root).unwrap();
        let page = valid_root.join("page.html");
        std::fs::write(&page, b"<body>valid grant</body>").unwrap();

        // The missing root is first on purpose: stale grants must not prevent
        // the transport from reaching a later, still-authorized directory.
        let allowed_roots = [missing_root, valid_root.clone()];
        let transport = LocalFileTransport::new().with_allowed_roots(allowed_roots);
        let request = Request::new(Url::parse(&local_file_test_url(&page)).unwrap());
        let response = transport
            .send(&request)
            .expect("a stale grant must not disable another valid root");
        assert_eq!(response.body, b"<body>valid grant</body>");

        std::fs::remove_file(page).unwrap();
        std::fs::remove_dir(valid_root).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_file_transport_rejects_final_component_symlinks() {
        use std::os::unix::fs::symlink;

        let directory =
            std::env::temp_dir().join(format!("archia-browser-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target.html");
        let link = directory.join("link.html");
        std::fs::write(&target, b"<body>must not follow</body>").unwrap();
        symlink(&target, &link).unwrap();

        let request = Request::new(Url::parse(&local_file_test_url(&link)).unwrap());
        assert_eq!(
            LocalFileTransport::new()
                .with_allowed_roots([directory.clone()])
                .send(&request),
            Err(TransportError::ConnectionFailed),
            "a local file URL must not follow a final-component symlink"
        );

        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn openat2_confines_file_open_to_the_authorized_root_handle() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-openat2-confinement-{}",
            std::process::id()
        ));
        let root_path = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root_path).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let allowed = root_path.join("allowed.txt");
        let secret = outside.join("secret.txt");
        std::fs::write(&allowed, b"allowed").unwrap();
        std::fs::write(&secret, b"outside-secret").unwrap();

        let root = std::fs::canonicalize(&root_path).unwrap();
        let mut opened = open_local_file_confined(&root, &allowed, &allowed)
            .expect("an ordinary file beneath the granted root should open");
        let mut contents = String::new();
        opened.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "allowed");

        // The target path is lexically below the root, but the intermediate
        // component redirects outside it. Exercise the final open directly so
        // this verifies openat2 itself, independently of pathname pre-checks.
        let link = root.join("escape");
        symlink(&outside, &link).unwrap();
        let escaped_path = link.join("secret.txt");
        assert_eq!(
            open_local_file_confined(&root, &escaped_path, &escaped_path).err(),
            Some(TransportError::ConnectionFailed),
            "openat2 must reject an intermediate symlink that escapes the root"
        );

        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(allowed).unwrap();
        std::fs::remove_file(secret).unwrap();
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    #[test]
    fn openat_component_walk_rejects_intermediate_symlinks() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-openat-confinement-{}",
            std::process::id()
        ));
        let root_path = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root_path).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"OUTSIDE-SECRET").unwrap();
        symlink(&outside, root_path.join("escape")).unwrap();

        let root = std::fs::canonicalize(&root_path).unwrap();
        let root_handle = open_local_root_handle_verified(&root).unwrap();
        let escaped_path = root.join("escape").join("secret.txt");
        assert_eq!(
            open_local_file_confined_from_handle(&root_handle, &root, &escaped_path).err(),
            Some(TransportError::ConnectionFailed),
            "openat component walking must reject an intermediate symlink instead of following it outside the granted root"
        );

        drop(root_handle);
        std::fs::remove_file(root.join("escape")).unwrap();
        std::fs::remove_file(outside.join("secret.txt")).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(root_path).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_file_transport_rejects_a_symlink_used_as_the_granted_root() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-symlink-root-{}",
            std::process::id()
        ));
        let real_root = base.join("real");
        let alias_root = base.join("alias");
        std::fs::create_dir_all(&real_root).unwrap();
        std::fs::write(real_root.join("page.txt"), b"must-not-be-granted").unwrap();
        symlink(&real_root, &alias_root).unwrap();

        let transport = LocalFileTransport::new().with_allowed_roots([alias_root.clone()]);
        assert!(
            transport.authorized_root_handles.is_empty(),
            "a symlink in the user-supplied grant path must not be hidden by canonicalization"
        );

        std::fs::remove_file(real_root.join("page.txt")).unwrap();
        std::fs::remove_file(alias_root).unwrap();
        std::fs::remove_dir(real_root).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn local_file_root_grant_does_not_follow_a_concurrent_symlink_swap() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::thread;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-root-grant-race-{}",
            std::process::id()
        ));
        let root = base.join("root");
        let parked = base.join("root-parked");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("page.txt"), b"inside").unwrap();
        std::fs::write(outside.join("page.txt"), b"OUTSIDE-SECRET").unwrap();

        let stop = Arc::new(AtomicBool::new(false));
        let exposed_outside = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_root = root.clone();
        let worker_parked = parked.clone();
        let worker_outside = outside.clone();
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                if std::fs::rename(&worker_root, &worker_parked).is_err() {
                    thread::yield_now();
                    continue;
                }
                let linked = symlink(&worker_outside, &worker_root).is_ok();
                if linked {
                    thread::yield_now();
                    let _ = std::fs::remove_file(&worker_root);
                }
                let _ = std::fs::rename(&worker_parked, &worker_root);
            }
        });

        let url = super::super::Url::parse(&local_file_test_url(&root.join("page.txt"))).unwrap();
        for _ in 0..500 {
            let transport = LocalFileTransport::new().with_allowed_roots([root.clone()]);
            if !transport.authorized_root_handles.is_empty() {
                if let Ok(response) = transport.send(&Request::new(url.clone())) {
                    if response.body != b"inside" {
                        exposed_outside.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }

        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        if parked.exists() {
            if root.exists() {
                let _ = std::fs::remove_file(&root);
            }
            std::fs::rename(&parked, &root).unwrap();
        }
        std::fs::remove_file(root.join("page.txt")).unwrap();
        std::fs::remove_file(outside.join("page.txt")).unwrap();
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(base).unwrap();
        assert!(
            !exposed_outside.load(Ordering::Relaxed),
            "a root grant racing with a symlink swap must never expose outside content"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn local_file_transport_keeps_the_original_granted_root_handle() {
        let base =
            std::env::temp_dir().join(format!("archia-browser-pinned-root-{}", std::process::id()));
        let root = base.join("granted");
        let moved_root = base.join("granted-original");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("page.txt"), b"authorized-original").unwrap();

        let url = super::super::Url::parse(&local_file_test_url(&root.join("page.txt"))).unwrap();
        let transport = LocalFileTransport::new().with_allowed_roots([root.clone()]);

        // Replace the configured pathname after granting access. The transport
        // must continue to resolve against its pinned directory handle, not the
        // new object that happens to occupy the same pathname.
        std::fs::rename(&root, &moved_root).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("page.txt"), b"replacement-secret").unwrap();

        let response = transport.send(&Request::new(url)).unwrap();
        assert_eq!(response.body, b"authorized-original");

        std::fs::remove_file(root.join("page.txt")).unwrap();
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_file(moved_root.join("page.txt")).unwrap();
        std::fs::remove_dir(moved_root).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_handle_relative_open_rejects_directory_junctions() {
        use std::process::Command;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-windows-junction-{}",
            std::process::id()
        ));
        let root_path = base.join("root");
        let outside = base.join("outside");
        let junction = root_path.join("escape");
        std::fs::create_dir_all(&root_path).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"OUTSIDE-SECRET").unwrap();

        // Junctions do not require the Developer Mode symlink privilege.
        let output = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .expect("Windows cmd.exe must be available to create a junction");
        assert!(
            output.status.success(),
            "could not create test junction: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let canonical_root = std::fs::canonicalize(&root_path).unwrap();
        let root_handle = open_local_root_handle_verified(&canonical_root).unwrap();
        let escaped_path = canonical_root.join("escape").join("secret.txt");
        assert_eq!(
            open_local_file_confined_from_handle(&root_handle, &canonical_root, &escaped_path)
                .err(),
            Some(TransportError::ConnectionFailed),
            "handle-relative opening must reject an intermediate Windows directory junction"
        );

        drop(root_handle);
        std::fs::remove_dir(&junction).unwrap();
        std::fs::remove_file(outside.join("secret.txt")).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(root_path).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn openat2_does_not_escape_during_concurrent_parent_swap() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-openat2-race-{}",
            std::process::id()
        ));
        let root_path = base.join("root");
        let outside = base.join("outside");
        let parent = root_path.join("parent");
        let parked = root_path.join("parent-parked");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(parent.join("target.txt"), b"inside").unwrap();
        std::fs::write(outside.join("target.txt"), b"OUTSIDE-SECRET").unwrap();

        let root = std::fs::canonicalize(&root_path).unwrap();
        let target = root.join("parent").join("target.txt");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_parent = parent.clone();
        let worker_parked = parked.clone();
        let worker_outside = outside.clone();

        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                // Each transition deliberately creates a window where the
                // requested component is either absent or a symlink outside.
                // openat2 must fail closed or open the original in-root file.
                if std::fs::rename(&worker_parent, &worker_parked).is_err() {
                    std::thread::yield_now();
                    continue;
                }
                let linked = symlink(&worker_outside, &worker_parent).is_ok();
                if linked {
                    let _ = std::fs::remove_file(&worker_parent);
                }
                let _ = std::fs::rename(&worker_parked, &worker_parent);
            }
        });

        for _ in 0..500 {
            match open_local_file_confined(&root, &target, &target) {
                Ok(mut file) => {
                    let mut contents = Vec::new();
                    file.read_to_end(&mut contents).unwrap();
                    assert_ne!(
                        contents, b"OUTSIDE-SECRET",
                        "a concurrent parent-directory swap must never expose the outside file"
                    );
                    assert_eq!(contents, b"inside");
                }
                Err(TransportError::ConnectionFailed) => {}
                Err(error) => panic!("unexpected open error during race: {error:?}"),
            }
        }

        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        std::fs::remove_file(parent.join("target.txt")).unwrap();
        std::fs::remove_dir(parent).unwrap();
        std::fs::remove_file(outside.join("target.txt")).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    #[test]
    fn openat_component_walk_does_not_escape_during_concurrent_parent_swap() {
        use std::os::unix::fs::symlink;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let base =
            std::env::temp_dir().join(format!("archia-browser-openat-race-{}", std::process::id()));
        let root_path = base.join("root");
        let outside = base.join("outside");
        let parent = root_path.join("parent");
        let parked = root_path.join("parent-parked");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(parent.join("target.txt"), b"inside").unwrap();
        std::fs::write(outside.join("target.txt"), b"OUTSIDE-SECRET").unwrap();

        let root = std::fs::canonicalize(&root_path).unwrap();
        let root_handle = open_local_root_handle_verified(&root).unwrap();
        let target = root.join("parent").join("target.txt");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_parent = parent.clone();
        let worker_parked = parked.clone();
        let worker_outside = outside.clone();

        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                // Swap the requested directory for an outside symlink. The
                // descriptor-relative component walk must fail closed or keep
                // operating on the already-opened in-root directory.
                if std::fs::rename(&worker_parent, &worker_parked).is_err() {
                    std::thread::yield_now();
                    continue;
                }
                let linked = symlink(&worker_outside, &worker_parent).is_ok();
                if linked {
                    let _ = std::fs::remove_file(&worker_parent);
                }
                let _ = std::fs::rename(&worker_parked, &worker_parent);
            }
        });

        for _ in 0..300 {
            match open_local_file_confined_from_handle(&root_handle, &root, &target) {
                Ok(mut file) => {
                    let mut contents = Vec::new();
                    file.read_to_end(&mut contents).unwrap();
                    assert_eq!(
                        contents, b"inside",
                        "a concurrent parent-directory swap must never expose the outside file"
                    );
                }
                Err(TransportError::ConnectionFailed) => {}
                Err(error) => panic!("unexpected open error during race: {error:?}"),
            }
        }

        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        drop(root_handle);
        std::fs::remove_file(parent.join("target.txt")).unwrap();
        std::fs::remove_dir(parent).unwrap();
        std::fs::remove_file(outside.join("target.txt")).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(root).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_file_transport_rejects_parent_directory_symlinks() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "archia-browser-parent-symlink-{}",
            std::process::id()
        ));
        let real_directory = base.join("real");
        let link_directory = base.join("linked");
        std::fs::create_dir_all(&real_directory).unwrap();
        let target = real_directory.join("secret.html");
        std::fs::write(&target, b"must not follow parent symlink").unwrap();
        symlink(&real_directory, &link_directory).unwrap();

        let request = Request::new(
            Url::parse(&local_file_test_url(&link_directory.join("secret.html"))).unwrap(),
        );
        assert_eq!(
            LocalFileTransport::new()
                .with_allowed_roots([base.clone()])
                .send(&request),
            Err(TransportError::ConnectionFailed),
            "a local file URL must not traverse an existing parent-directory symlink"
        );

        std::fs::remove_file(target).unwrap();
        std::fs::remove_file(link_directory).unwrap();
        std::fs::remove_dir(real_directory).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[test]
    fn local_file_transport_denies_reads_without_an_explicit_root() {
        let directory =
            std::env::temp_dir().join(format!("archia-browser-local-deny-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("secret.txt");
        std::fs::write(&path, b"secret").unwrap();
        let request = Request::new(Url::parse(&local_file_test_url(&path)).unwrap());

        assert_eq!(
            LocalFileTransport::new().send(&request),
            Err(TransportError::ConnectionFailed)
        );

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn local_file_transport_rejects_files_outside_allowed_roots() {
        let base =
            std::env::temp_dir().join(format!("archia-browser-root-policy-{}", std::process::id()));
        let allowed = base.join("allowed");
        let outside = base.join("outside");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let path = outside.join("secret.txt");
        std::fs::write(&path, b"secret").unwrap();
        let request = Request::new(Url::parse(&local_file_test_url(&path)).unwrap());

        assert_eq!(
            LocalFileTransport::new()
                .with_allowed_roots([allowed.clone()])
                .send(&request),
            Err(TransportError::ConnectionFailed)
        );

        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(outside).unwrap();
        std::fs::remove_dir(allowed).unwrap();
        std::fs::remove_dir(base).unwrap();
    }

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn local_file_transport_fails_closed_without_handle_relative_confinement() {
        let directory = std::env::temp_dir().join(format!(
            "archia-browser-nonlinux-file-deny-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("page.txt");
        std::fs::write(&path, b"must not be exposed").unwrap();
        let request = Request::new(Url::parse(&local_file_test_url(&path)).unwrap());
        let result = LocalFileTransport::new()
            .with_allowed_roots([directory.clone()])
            .send(&request);
        assert_eq!(
            result,
            Err(TransportError::ConnectionFailed),
            "local file access must remain disabled until this platform has race-safe handle-relative opening"
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&directory);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn local_file_transport_loads_html() {
        let path =
            std::env::temp_dir().join(format!("archia-browser-local-{}.html", std::process::id()));
        std::fs::write(&path, b"<body>Local</body>").unwrap();
        let url = local_file_test_url(&path);
        let request = Request::new(super::super::Url::parse(&url).unwrap());
        let response = LocalFileTransport::new()
            .with_allowed_roots([path.parent().unwrap().to_path_buf()])
            .send(&request)
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.content_type.as_deref(),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(response.header("content-length"), Some("18"));
        assert_eq!(response.body, b"<body>Local</body>");

        let head_request = Request::new(super::super::Url::parse(&url).unwrap())
            .with_method(crate::net::HttpMethod::Head);
        let head_response = LocalFileTransport::new()
            .with_allowed_roots([path.parent().unwrap().to_path_buf()])
            .send(&head_request)
            .unwrap();
        assert_eq!(head_response.header("content-length"), Some("18"));
        assert!(head_response.body.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn local_file_transport_enforces_size_limit_for_get_and_head() {
        let path = std::env::temp_dir().join(format!(
            "archia-browser-local-limit-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, b"12345").unwrap();
        let url = local_file_test_url(&path);
        let request = Request::new(super::super::Url::parse(&url).unwrap());
        let transport = LocalFileTransport::new()
            .with_allowed_roots([path.parent().unwrap().to_path_buf()])
            .with_max_file_size(4);
        assert_eq!(
            transport.send(&request),
            Err(TransportError::ResponseTooLarge)
        );
        let head = request.clone().with_method(crate::net::HttpMethod::Head);
        assert_eq!(transport.send(&head), Err(TransportError::ResponseTooLarge));
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn local_file_transport_rejects_relative_paths() {
        for path in ["relative.txt", "./relative.txt", "../outside.txt"] {
            assert_eq!(
                file_url_path(path),
                Err(TransportError::InvalidRequest),
                "relative file URL path must not depend on process working directory: {path}"
            );
        }
        assert_eq!(
            file_url_path("/tmp/absolute.txt").unwrap(),
            std::path::PathBuf::from("/tmp/absolute.txt")
        );
    }

    #[test]
    fn local_file_transport_rejects_ambiguous_decoded_path_structure() {
        for path in [
            "/tmp/../etc/passwd",
            "/tmp/./secret.txt",
            "/tmp/%2e%2e/etc/passwd",
            "/tmp/%2E/secret.txt",
            "/tmp/private%2fsecret.txt",
            "/tmp/private%5Csecret.txt",
            "/tmp/private\\\\secret.txt",
        ] {
            assert!(
                file_url_path(path).is_err(),
                "ambiguous or traversing local path must be rejected: {path}"
            );
        }
    }

    #[test]
    fn local_file_transport_rejects_malformed_percent_encoding() {
        for path in ["/tmp/%", "/tmp/%2", "/tmp/%GG", "/tmp/%FF"] {
            assert!(
                file_url_path(path).is_err(),
                "malformed file URL path should be rejected: {path}"
            );
        }
        assert!(file_url_path("/tmp/%00secret").is_err());
    }

    #[test]
    fn local_file_transport_rejects_nul_after_percent_decoding() {
        for path in ["/tmp/safe%00hidden", "/tmp/%00", "/tmp/safe%00/../secret"] {
            assert_eq!(
                file_url_path(path),
                Err(TransportError::InvalidUrl(
                    super::super::UrlError::InvalidCharacter
                )),
                "decoded NUL must never reach filesystem APIs: {path}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn local_file_transport_decodes_percent_escapes_exactly_once() {
        #[cfg(windows)]
        let (input, expected) = ("/C:/tmp/%252e%252e/secret.txt", "C:/tmp/%2e%2e/secret.txt");
        #[cfg(not(windows))]
        let (input, expected) = ("/tmp/%252e%252e/secret.txt", "/tmp/%2e%2e/secret.txt");

        let decoded = file_url_path(input).unwrap();
        assert_eq!(
            decoded,
            std::path::PathBuf::from(expected),
            "double-encoded dot segments must remain literal after one URL decode"
        );
    }

    #[cfg(windows)]
    #[test]
    fn local_file_transport_rejects_unc_and_drive_relative_paths() {
        for path in [
            "//server/share/secret.txt",
            "C:secret.txt",
            "/C:secret.txt",
            "/C:/safe/file:secret.txt",
            "/C:/safe/file%3Asecret.txt",
            "C:/safe/file:secret.txt",
            "relative.txt",
        ] {
            assert!(
                file_url_path(path).is_err(),
                "ambiguous or network-backed Windows file path must be rejected: {path}"
            );
        }

        assert_eq!(
            file_url_path("/C:/Windows/system.ini").unwrap(),
            std::path::PathBuf::from("C:/Windows/system.ini")
        );
    }

    #[test]
    fn local_file_transport_rejects_directories() {
        let path = std::env::temp_dir();
        let url = local_file_test_url(&path);
        let request = Request::new(super::super::Url::parse(&url).unwrap());
        assert_eq!(
            LocalFileTransport::new().send(&request),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn local_file_transport_rejects_non_file_urls() {
        let request = Request::new(super::super::Url::parse("https://example.org/").unwrap());
        assert_eq!(
            LocalFileTransport::new().send(&request).unwrap_err(),
            TransportError::UnsupportedScheme
        );
    }

    #[test]
    fn local_file_transport_never_receives_encoded_traversal_or_separators() {
        // Keep the URL parser and the filesystem transport aligned: encoded
        // dot segments and encoded path separators must be rejected before
        // a local filesystem path can be opened.
        for input in [
            "file:///tmp/%2e%2e/etc/passwd",
            "file:///tmp/%2E%2E/etc/passwd",
            "file:///tmp/private%2fsecret.txt",
            "file:///tmp/private%2Fsecret.txt",
        ] {
            assert!(
                super::super::Url::parse(input).is_err(),
                "unsafe file URL must be rejected before transport: {input}"
            );
        }
    }

    #[test]
    fn local_file_transport_rejects_remote_authorities_and_userinfo() {
        // Remote file authorities can cause platform-dependent network-share
        // access (notably UNC paths on Windows). Userinfo must not turn a
        // remote authority into an accepted local-file request either.
        for input in [
            "file://server/share/secret.txt",
            "file://192.0.2.1/share/secret.txt",
            "file://user:secret@localhost/tmp/secret.txt",
            "file://attacker.example/tmp/secret.txt",
        ] {
            assert!(
                super::super::Url::parse(input).is_err(),
                "remote or credential-bearing file URL must be rejected during parsing: {input}"
            );
        }

        for input in ["file:///tmp/secret.txt", "file://localhost/tmp/secret.txt"] {
            let url = super::super::Url::parse(input).expect("local file URL should parse");
            assert!(
                url.authority().is_empty() || url.authority().eq_ignore_ascii_case("localhost"),
                "only an empty authority or localhost is considered local"
            );
        }
    }

    #[test]
    fn local_file_transport_rejects_methods_and_bodies_outside_get_head() {
        let path = std::env::temp_dir().join(format!(
            "archia-browser-local-method-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, b"must not be uploaded").unwrap();
        let url = super::super::Url::parse(&local_file_test_url(&path)).unwrap();

        let post = Request::new(url.clone())
            .with_method(crate::net::HttpMethod::Post)
            .with_body(b"request payload".to_vec());
        assert_eq!(
            LocalFileTransport::new().send(&post),
            Err(TransportError::InvalidRequest)
        );

        let get_with_body = Request::new(url).with_body(b"ambiguous payload".to_vec());
        assert_eq!(
            LocalFileTransport::new().send(&get_with_body),
            Err(TransportError::InvalidRequest)
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_malformed_ports_and_user_info_before_connecting() {
        for input in [
            "http://example.org:bad/",
            "http://example.org:99999/",
            "http://example.org:/",
            "http://example.org:80:90/",
        ] {
            assert!(
                Url::parse(input).is_err(),
                "expected malformed URL authority to be rejected: {input}"
            );
        }

        let request = Request::new(Url::parse("http://user@example.org/").unwrap());
        assert_eq!(
            HttpTransport::new().send(&request),
            Err(TransportError::InvalidRequest),
            "userinfo must be rejected by the HTTP transport"
        );
    }

    #[test]
    fn accepts_absolute_dns_hostnames_with_one_trailing_dot() {
        let request = Request::new(Url::parse("http://example.org./").unwrap());
        assert_eq!(validate_request(&request), Ok(()));
        assert_eq!(host_header(&request), "example.org.");
    }

    #[test]
    fn rejects_empty_dns_labels_even_with_trailing_dots() {
        for input in [
            "http://example.org../",
            "http://example..org./",
            "http://./",
        ] {
            assert!(
                Url::parse(input).is_err(),
                "expected empty DNS labels to be rejected: {input}"
            );
        }
    }

    #[test]
    fn accepts_valid_ipv6_authorities() {
        let request = Request::new(Url::parse("http://[::1]:8080/").unwrap());
        assert_eq!(validate_request(&request), Ok(()));
    }

    #[test]
    fn rejects_authority_injection() {
        assert_eq!(
            Url::parse("http://example.org evil/"),
            Err(crate::net::url::UrlError::InvalidCharacter)
        );

        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("Host", "example.org evil");
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_transfer_encoding_on_http_10_responses() {
        let response = b"HTTP/1.0 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_conflicting_transfer_encoding_and_content_length() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nHello\r\n0\r\n\r\n",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn forces_connection_close_even_if_request_asks_for_keep_alive() {
        #[derive(Debug, Default)]
        struct CaptureStream {
            written: Vec<u8>,
        }

        impl Read for CaptureStream {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }

        impl Write for CaptureStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.written.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("connection", "keep-alive");
        let transport = HttpTransport::new();
        let mut stream = CaptureStream::default();
        transport.write_request(&mut stream, &request).unwrap();

        let head = String::from_utf8(stream.written).unwrap();
        assert_eq!(head.matches("Connection: close").count(), 1);
        assert!(!head.to_ascii_lowercase().contains("connection: keep-alive"));
    }

    #[test]
    fn rejects_proxy_credentials_on_direct_origin_transport() {
        for name in ["proxy-authorization", "Proxy-Authorization"] {
            let mut request = Request::new(Url::parse("https://example.org/").unwrap());
            request
                .headers
                .insert(name.to_owned(), "Basic secret".to_owned());
            assert_eq!(
                validate_request(&request),
                Err(TransportError::InvalidRequest),
                "proxy credentials must never be sent to an origin"
            );
        }
    }

    #[test]
    fn rejects_connection_nominated_request_headers_and_malformed_tokens() {
        let url = Url::parse("http://example.org/").unwrap();
        for request in [
            Request::new(url.clone())
                .with_header("connection", "x-internal")
                .with_header("x-internal", "secret"),
            Request::new(url.clone()).with_header("connection", "close,,keep-alive"),
            Request::new(url.clone()).with_header("connection", "bad token"),
        ] {
            assert_eq!(
                validate_request(&request),
                Err(TransportError::InvalidRequest)
            );
        }

        let generated_host = Request::new(url.clone()).with_header("connection", "host");
        assert_eq!(
            validate_request(&generated_host),
            Err(TransportError::InvalidRequest)
        );
        let generated_length = Request::new(url.clone())
            .with_header("connection", "content-length")
            .with_body(b"payload".to_vec());
        assert_eq!(
            validate_request(&generated_length),
            Err(TransportError::InvalidRequest)
        );

        let valid = Request::new(url).with_header("connection", "close");
        assert_eq!(validate_request(&valid), Ok(()));
    }

    #[test]
    fn rejects_invalid_request_header_names_and_mismatched_host() {
        let invalid_name = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("Bad Header", "value");
        assert_eq!(
            validate_request(&invalid_name),
            Err(TransportError::InvalidRequest)
        );

        let mismatched_host = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("host", "attacker.example");
        assert_eq!(
            validate_request(&mismatched_host),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_ambiguous_request_body_framing() {
        let url = Url::parse("http://example.org/").unwrap();

        let short_length = Request::new(url.clone())
            .with_method(crate::net::HttpMethod::Post)
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "4");
        assert_eq!(
            validate_request(&short_length),
            Err(TransportError::InvalidRequest)
        );

        let invalid_length = Request::new(url.clone())
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "five");
        assert_eq!(
            validate_request(&invalid_length),
            Err(TransportError::InvalidRequest)
        );

        for invalid in ["+5", "-0", ""] {
            let request = Request::new(url.clone())
                .with_body(b"Hello".to_vec())
                .with_header("content-length", invalid);
            assert_eq!(
                validate_request(&request),
                Err(TransportError::InvalidRequest),
                "invalid Content-Length syntax must be rejected: {invalid:?}"
            );
        }

        let transfer_encoded = Request::new(url.clone())
            .with_body(b"Hello".to_vec())
            .with_header("transfer-encoding", "chunked");
        assert_eq!(
            validate_request(&transfer_encoded),
            Err(TransportError::InvalidRequest)
        );

        let mut duplicate_length = Request::new(url)
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "5");
        // Construct a malformed map directly: the public builder normalizes
        // names and would otherwise collapse this duplicate before validation.
        duplicate_length
            .headers
            .insert("Content-Length".into(), "5".into());
        assert_eq!(
            validate_request(&duplicate_length),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn validates_mixed_case_host_headers_case_insensitively() {
        let mut request = Request::new(Url::parse("http://example.org/").unwrap());
        request
            .headers
            .insert("Host".into(), "attacker.example".into());
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest)
        );

        request.headers.insert("Host".into(), "example.org".into());
        assert_eq!(validate_request(&request), Ok(()));
    }

    #[test]
    fn does_not_emit_duplicate_content_length_for_mixed_case_header() {
        #[derive(Debug, Default)]
        struct CaptureStream {
            written: Vec<u8>,
        }

        impl Read for CaptureStream {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }

        impl Write for CaptureStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.written.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_method(super::super::HttpMethod::Post)
            .with_body(b"Hello".to_vec());
        request.headers.insert("Content-Length".into(), "5".into());
        let mut stream = CaptureStream::default();
        HttpTransport::new()
            .write_request(&mut stream, &request)
            .unwrap();

        let head = String::from_utf8(stream.written).unwrap();
        assert_eq!(
            head.lines()
                .filter(|line| line
                    .split_once(':')
                    .is_some_and(|(name, _)| { name.eq_ignore_ascii_case("content-length") }))
                .count(),
            1,
            "a mixed-case caller header must not trigger a second Content-Length"
        );
    }

    #[test]
    fn content_length_uses_only_http_ascii_optional_whitespace() {
        let url = Url::parse("http://example.org/").unwrap();

        let mut request = Request::new(url.clone())
            .with_method(super::super::HttpMethod::Post)
            .with_body(b"Hello".to_vec());
        request
            .headers
            .insert("Content-Length".into(), "\u{00a0}5\u{00a0}".into());
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest),
            "non-ASCII whitespace must not be silently stripped from wire framing"
        );

        let valid_ows = Request::new(url)
            .with_method(super::super::HttpMethod::Post)
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "\t5 \t");
        assert_eq!(validate_request(&valid_ows), Ok(()));

        let response = b"HTTP/1.1 200 OK\r\nContent-Length: \xC2\xA05\xC2\xA0\r\n\r\nHello";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed),
            "response framing must use the same ASCII OWS rules"
        );
    }

    #[test]
    fn rejects_mixed_case_request_framing_headers() {
        let url = Url::parse("http://example.org/").unwrap();

        let mut transfer_encoded = Request::new(url.clone()).with_body(b"Hello".to_vec());
        transfer_encoded
            .headers
            .insert("Transfer-Encoding".into(), "chunked".into());
        assert_eq!(
            validate_request(&transfer_encoded),
            Err(TransportError::InvalidRequest)
        );

        let mut invalid_length = Request::new(url.clone()).with_body(b"Hello".to_vec());
        invalid_length
            .headers
            .insert("Content-Length".into(), "4".into());
        assert_eq!(
            validate_request(&invalid_length),
            Err(TransportError::InvalidRequest)
        );

        let mut duplicate_length = Request::new(url).with_body(b"Hello".to_vec());
        duplicate_length
            .headers
            .insert("content-length".into(), "5".into());
        duplicate_length
            .headers
            .insert("Content-Length".into(), "5".into());
        assert_eq!(
            validate_request(&duplicate_length),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn request_header_block_limit_covers_n_minus_one_n_and_n_plus_one() {
        let url = Url::parse("http://example.org/").unwrap();
        // For this URL and one "x-large" header, all fixed request-line,
        // Host, Connection and delimiter bytes total 67 bytes.
        let fixed_bytes = 67;
        for (value_len, expected) in [
            (MAX_REQUEST_HEADER_BYTES - fixed_bytes - 1, Ok(())),
            (MAX_REQUEST_HEADER_BYTES - fixed_bytes, Ok(())),
            (
                MAX_REQUEST_HEADER_BYTES - fixed_bytes + 1,
                Err(TransportError::InvalidRequest),
            ),
        ] {
            let request = Request::new(url.clone()).with_header("x-large", "a".repeat(value_len));
            assert_eq!(
                validate_request(&request),
                expected,
                "unexpected validation result for serialized header size {}",
                fixed_bytes + value_len
            );
        }
    }

    #[test]
    fn rejects_oversized_request_header_block_before_connecting() {
        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("x-large", "a".repeat(MAX_REQUEST_HEADER_BYTES));
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest)
        );

        assert!(matches!(
            Url::parse(&format!(
                "http://example.org/{}",
                "a".repeat(MAX_REQUEST_HEADER_BYTES)
            )),
            Err(super::super::UrlError::TooLong)
        ));
    }

    #[test]
    fn rejects_header_injection() {
        let transport = HttpTransport::new();
        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("x-test", "safe\r\nX-Injected: yes");
        assert_eq!(
            transport.send(&request),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn formats_ipv6_socket_addresses_with_brackets() {
        let url = Url::parse("http://[::1]:8080/").unwrap();
        assert_eq!(socket_address(&url), "[::1]:8080");
    }

    #[test]
    fn omits_default_ports_from_http_and_https_host_headers() {
        let http = Request::new(Url::parse("http://example.org:80/").unwrap());
        assert_eq!(host_header(&http), "example.org");

        let https = Request::new(Url::parse("https://example.org/").unwrap());
        assert_eq!(host_header(&https), "example.org");

        let explicit_https_port = Request::new(Url::parse("https://example.org:8443/").unwrap());
        assert_eq!(host_header(&explicit_https_port), "example.org:8443");
    }

    #[test]
    fn builds_default_and_explicit_host_headers() {
        let request = Request::new(Url::parse("http://example.org/").unwrap());
        assert_eq!(host_header(&request), "example.org");

        let request = Request::new(Url::parse("http://example.org:8080/").unwrap());
        assert_eq!(host_header(&request), "example.org:8080");

        let request = Request::new(Url::parse("http://[::1]:8080/").unwrap());
        assert_eq!(host_header(&request), "[::1]:8080");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use crate::net::Url;

    #[test]
    fn http_transport_round_trips_a_response_over_a_real_loopback_socket() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap();
                assert_ne!(count, 0, "client closed before sending HTTP headers");
                request.extend_from_slice(&chunk[..count]);
                assert!(request.len() <= MAX_REQUEST_HEADER_BYTES);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nHello")
                .unwrap();
            request
        });

        let request = Request::new(
            Url::parse(&format!("http://127.0.0.1:{}/probe?q=1", address.port())).unwrap(),
        );
        let response = HttpTransport::with_timeouts(Duration::from_secs(2), Duration::from_secs(2))
            .send(&request)
            .expect("valid loopback HTTP response should parse");
        let wire_request = server.join().unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"Hello");
        assert_eq!(response.content_type.as_deref(), Some("text/plain"));
        let wire_request = String::from_utf8(wire_request).unwrap();
        assert!(wire_request.starts_with("GET /probe?q=1 HTTP/1.1\r\n"));
        assert!(wire_request.contains("Connection: close\r\n"));
    }

    #[test]
    fn http_transport_rejects_ambiguous_and_truncated_framing_over_loopback() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let malformed_responses: &[&[u8]] = &[
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\nx",
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nx",
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nxEXTRA",
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: +1\r\n\r\nx",
        ];

        for malformed in malformed_responses {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let address = listener.local_addr().unwrap();
            let response_bytes = malformed.to_vec();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request).unwrap();
                stream.write_all(&response_bytes).unwrap();
            });

            let request =
                Request::new(Url::parse(&format!("http://127.0.0.1:{}/", address.port())).unwrap());
            assert_eq!(
                HttpTransport::with_timeouts(Duration::from_secs(2), Duration::from_secs(2),)
                    .send(&request),
                Err(TransportError::ConnectionFailed),
                "malformed wire response must fail closed: {:?}",
                String::from_utf8_lossy(malformed)
            );
            server.join().unwrap();
        }
    }

    #[test]
    fn tls_handshake_rejects_non_tls_peer_as_tls_failure() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut client_hello = [0_u8; 4096];
            let _ = stream.read(&mut client_hello).unwrap();
            stream.write_all(b"not a TLS record").unwrap();
        });

        let request =
            Request::new(Url::parse(&format!("https://127.0.0.1:{}/", address.port())).unwrap());
        assert_eq!(
            HttpTransport::with_timeouts(Duration::from_secs(2), Duration::from_secs(2))
                .send(&request),
            Err(TransportError::TlsFailed)
        );
        server.join().unwrap();
    }

    #[test]
    fn tls_server_name_accepts_dns_and_ip_literals() {
        assert!(tls_server_name("example.org").is_ok());
        assert!(tls_server_name("127.0.0.1").is_ok());
        assert!(tls_server_name("::1").is_ok());
    }

    #[test]
    fn tls_server_name_rejects_invalid_dns_names() {
        for host in ["", "bad host", "example.org/path", "example.org:443"] {
            assert!(
                matches!(tls_server_name(host), Err(TransportError::TlsFailed)),
                "{host:?}"
            );
        }
    }

    #[test]
    fn rejects_head_response_with_unexpected_wire_body() {
        let result = parse_http_response_for_method(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello",
            super::super::HttpMethod::Head,
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_zero_content_length_mismatch_and_truncated_fixed_body() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHell",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_body_forbidden_status_with_wire_body() {
        for status in [204, 205, 304] {
            let response = format!("HTTP/1.1 {status} No Content\r\n\r\nunexpected");
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "status {status}"
            );
        }
    }

    #[test]
    fn rejects_unsupported_protocol_switching_response() {
        assert_eq!(
            parse_http_response(
                b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n",
                1024,
                1024,
            ),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_excessive_interim_responses() {
        let mut response = Vec::new();
        for _ in 0..=MAX_INTERIM_RESPONSES {
            response.extend_from_slice(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        response.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(
            parse_http_response(&response, 1024, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_incomplete_chunk_terminator() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\n",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_oversized_headers_immediately_when_delimiter_arrives_in_same_read() {
        use std::io;

        struct CountingStream {
            bytes: Vec<u8>,
            offset: usize,
            reads: usize,
        }

        impl Read for CountingStream {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                if self.offset == self.bytes.len() {
                    return Ok(0);
                }
                let count = output.len().min(self.bytes.len() - self.offset);
                output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                Ok(count)
            }
        }

        impl Write for CountingStream {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut wire = b"HTTP/1.1 200 OK\r\nX-Large: ".to_vec();
        wire.extend(std::iter::repeat_n(b'a', 16_400));
        wire.extend_from_slice(b"\r\n\r\nBODY");
        let mut stream = CountingStream {
            bytes: wire,
            offset: 0,
            reads: 0,
        };
        let transport = HttpTransport::new().with_limits(1024 * 1024, 16 * 1024);

        assert_eq!(
            transport.read_response(
                &mut stream,
                super::super::HttpMethod::Get,
                &CancellationToken::new(),
            ),
            Err(TransportError::ResponseTooLarge)
        );
        // The oversized header is rejected on the read that reveals its end,
        // without another read to EOF or buffering the rest of the response.
        assert_eq!(stream.reads, 2);
    }

    #[test]
    fn response_header_limit_accepts_exact_boundary_and_rejects_one_byte_over() {
        let prefix = b"HTTP/1.1 200 OK\r\nX-Large: ";
        let header_limit = 1024;
        let transport = HttpTransport::new().with_limits(4096, header_limit);

        for header_bytes in [header_limit - 1, header_limit] {
            let mut wire = prefix.to_vec();
            wire.extend(std::iter::repeat_n(b'a', header_bytes - prefix.len()));
            wire.extend_from_slice(b"\r\n\r\nBODY");
            let mut stream = std::io::Cursor::new(wire);
            let response = transport
                .read_response(
                    &mut stream,
                    super::super::HttpMethod::Get,
                    &CancellationToken::new(),
                )
                .expect("headers at or below the configured limit must be accepted");
            assert_eq!(response.body, b"BODY");
        }

        let mut wire = prefix.to_vec();
        wire.extend(std::iter::repeat_n(b'a', header_limit + 1 - prefix.len()));
        wire.extend_from_slice(b"\r\n\r\nBODY");
        let mut stream = std::io::Cursor::new(wire);
        assert_eq!(
            transport.read_response(
                &mut stream,
                super::super::HttpMethod::Get,
                &CancellationToken::new(),
            ),
            Err(TransportError::ResponseTooLarge),
            "one byte over the header limit must be rejected"
        );
    }

    #[test]
    fn exact_limit_header_survives_delimiter_split_across_reads() {
        use std::io;

        struct ChunkedStream {
            bytes: Vec<u8>,
            offset: usize,
            chunk_size: usize,
        }

        impl Read for ChunkedStream {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                if self.offset == self.bytes.len() {
                    return Ok(0);
                }
                let count = output
                    .len()
                    .min(self.chunk_size)
                    .min(self.bytes.len() - self.offset);
                output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                Ok(count)
            }
        }

        impl Write for ChunkedStream {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let prefix = b"HTTP/1.1 200 OK\r\nX-Large: ";
        let header_limit = 1024;
        let mut wire = prefix.to_vec();
        wire.extend(std::iter::repeat_n(b'a', header_limit - prefix.len()));
        wire.extend_from_slice(b"\r\n\r\nBODY");
        let mut stream = ChunkedStream {
            bytes: wire,
            offset: 0,
            // The first read includes the full header and only the first byte
            // of the four-byte delimiter.
            chunk_size: header_limit + 1,
        };
        let transport = HttpTransport::new().with_limits(4096, header_limit);

        let response = transport
            .read_response(
                &mut stream,
                super::super::HttpMethod::Get,
                &CancellationToken::new(),
            )
            .expect("a partial delimiter must not make an exact-limit header oversized");
        assert_eq!(response.body, b"BODY");
    }

    #[test]
    fn request_framing_rejects_mismatched_content_length_and_transfer_encoding() {
        let mut mismatched = Request::new(Url::parse("http://127.0.0.1/upload").unwrap())
            .with_method(super::super::HttpMethod::Post)
            .with_header("Content-Length", "2");
        mismatched.body = b"abc".to_vec();
        assert_eq!(
            validate_request(&mismatched),
            Err(TransportError::InvalidRequest)
        );

        let mut unsupported = Request::new(Url::parse("http://127.0.0.1/upload").unwrap())
            .with_method(super::super::HttpMethod::Post)
            .with_header("Transfer-Encoding", "chunked");
        unsupported.body = b"3\\r\\nabc\\r\\n0\\r\\n\\r\\n".to_vec();
        assert_eq!(
            validate_request(&unsupported),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn response_framing_rejects_ambiguous_or_unsupported_transfer_codings() {
        for wire in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked, chunked\r\n\r\n0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: +0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 0, 0\r\n\r\n"[..],
        ] {
            assert_eq!(
                parse_http_response(wire, 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "ambiguous response framing must fail closed: {:?}",
                String::from_utf8_lossy(wire)
            );
        }
    }

    #[test]
    fn response_reader_rejects_projected_size_before_appending_read_chunk() {
        use std::io;

        struct CountingStream {
            bytes: Vec<u8>,
            offset: usize,
            reads: usize,
        }

        impl Read for CountingStream {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.reads += 1;
                if self.offset == self.bytes.len() {
                    return Ok(0);
                }
                let count = output.len().min(self.bytes.len() - self.offset);
                output[..count].copy_from_slice(&self.bytes[self.offset..self.offset + count]);
                self.offset += count;
                Ok(count)
            }
        }

        impl Write for CountingStream {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut wire = b"HTTP/1.1 200 OK\\r\\nContent-Length: 100\\r\\n\\r\\n".to_vec();
        wire.extend(std::iter::repeat_n(b'x', 100));
        let mut stream = CountingStream {
            bytes: wire,
            offset: 0,
            reads: 0,
        };
        let transport = HttpTransport::new().with_limits(64, 1024);

        assert_eq!(
            transport.read_response(
                &mut stream,
                super::super::HttpMethod::Get,
                &CancellationToken::new(),
            ),
            Err(TransportError::ResponseTooLarge)
        );
        // The transport rejects the first chunk whose projected length exceeds
        // the cap; it must not keep reading the rest of the peer's response.
        assert_eq!(stream.reads, 1);
    }

    #[test]
    fn socket_io_timeouts_are_consistent_for_reads_and_writes() {
        assert_eq!(
            map_transport_io_error(std::io::Error::from(std::io::ErrorKind::TimedOut)),
            TransportError::Timeout
        );
        assert_eq!(
            map_transport_io_error(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
            TransportError::Timeout
        );
        assert_eq!(
            map_transport_io_error(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
            TransportError::ConnectionFailed
        );
    }

    #[test]
    fn connect_timeout_is_not_collapsed_into_connection_failure() {
        assert_eq!(connect_failure_error(true), TransportError::Timeout);
        assert_eq!(
            connect_failure_error(false),
            TransportError::ConnectionFailed
        );
        assert!(is_timeout_error(std::io::ErrorKind::TimedOut));
        assert!(is_timeout_error(std::io::ErrorKind::WouldBlock));
        assert!(!is_timeout_error(std::io::ErrorKind::ConnectionRefused));
    }

    #[test]
    fn tls_handshake_io_timeouts_are_reported_as_timeouts() {
        assert_eq!(
            map_tls_io_error(std::io::Error::from(std::io::ErrorKind::TimedOut)),
            TransportError::Timeout
        );
        assert_eq!(
            map_tls_io_error(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
            TransportError::Timeout
        );
        assert_eq!(
            map_tls_io_error(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
            TransportError::TlsFailed
        );
    }

    #[test]
    fn rejects_oversized_content_length() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello",
            4,
            1024,
        );
        assert_eq!(result, Err(TransportError::ResponseTooLarge));
    }

    #[test]
    fn rejects_chunk_size_that_overflows_input_offset() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nFFFFFFFFFFFFFFFF\r\n";
        assert_eq!(
            parse_http_response(response, usize::MAX, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_oversized_chunked_body() {
        let under = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n0\r\n\r\n",
            3,
            1024,
        )
        .unwrap();
        assert_eq!(under.body, b"ab");

        let exact = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
            3,
            1024,
        )
        .unwrap();
        assert_eq!(exact.body, b"abc");

        let over = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n0\r\n\r\n",
            3,
            1024,
        );
        assert_eq!(over, Err(TransportError::ResponseTooLarge));
    }

    #[test]
    fn response_body_limit_accepts_exact_boundary_and_rejects_one_byte_over() {
        let under = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nab";
        assert_eq!(parse_http_response(under, 3, 1024).unwrap().body, b"ab");

        let exact = b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabc";
        let response = parse_http_response(exact, 3, 1024).unwrap();
        assert_eq!(response.body, b"abc");
        assert_eq!(
            parse_http_response(exact, 2, 1024),
            Err(TransportError::ResponseTooLarge)
        );

        let over = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabcd";
        assert_eq!(
            parse_http_response(over, 3, 1024),
            Err(TransportError::ResponseTooLarge)
        );
    }

    #[test]
    fn response_header_limit_accepts_exact_boundary_and_rejects_one_byte_under() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
        let separator = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();

        assert_eq!(
            parse_http_response(response, 1024, separator)
                .unwrap()
                .status,
            200
        );
        assert_eq!(
            parse_http_response(response, 1024, separator - 1),
            Err(TransportError::ResponseTooLarge)
        );
    }

    #[test]
    fn rejects_oversized_chunk_size_line() {
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;".to_vec();
        response.extend(std::iter::repeat_n(b'a', 256));
        response.extend_from_slice(b"\r\nx\r\n0\r\n\r\n");

        assert_eq!(
            parse_http_response(&response, 1024, 128),
            Err(TransportError::ResponseTooLarge)
        );
    }
}
