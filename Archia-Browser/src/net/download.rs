//! Bounded HTTP downloads with redirect validation and atomic file publication.

use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use super::{HttpMethod, Request, Transport, TransportError, Url};

const MAX_REDIRECTS: usize = 10;
const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadResult {
    pub final_url: Url,
    pub destination: PathBuf,
    pub suggested_filename: String,
    pub content_type: Option<String>,
    pub bytes_written: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    Network(TransportError),
    UnsupportedScheme,
    HttpStatus(u16),
    RedirectLimitExceeded,
    RedirectLoopDetected,
    InvalidRedirect,
    SecureDowngradeBlocked,
    ResponseTooLarge,
    DestinationExists,
    Io(String),
}

/// Fetch a URL and publish its response body to the selected destination.
/// The default body ceiling is 64 MiB.
pub fn download_to_path<T: Transport>(
    transport: &T,
    request: &Request,
    destination: impl AsRef<Path>,
) -> Result<DownloadResult, DownloadError> {
    download_to_path_with_limit(transport, request, destination, DEFAULT_MAX_DOWNLOAD_BYTES)
}

/// Variant of download_to_path with an explicit response-body ceiling.
pub fn download_to_path_with_limit<T: Transport>(
    transport: &T,
    request: &Request,
    destination: impl AsRef<Path>,
    max_bytes: usize,
) -> Result<DownloadResult, DownloadError> {
    let destination = destination.as_ref();
    if destination.exists() {
        return Err(DownloadError::DestinationExists);
    }

    let mut current = request.clone();
    current.method = HttpMethod::Get;
    current.body.clear();
    let mut visited = HashSet::new();

    for redirect_count in 0..=MAX_REDIRECTS {
        if !matches!(current.url.scheme(), "http" | "https") {
            return Err(DownloadError::UnsupportedScheme);
        }
        if !visited.insert(current.url.to_string()) {
            return Err(DownloadError::RedirectLoopDetected);
        }

        let response = transport.send(&current).map_err(DownloadError::Network)?;

        if is_redirect(response.status) {
            if redirect_count == MAX_REDIRECTS {
                return Err(DownloadError::RedirectLimitExceeded);
            }
            let location = response
                .header("location")
                .filter(|value| !value.trim().is_empty())
                .ok_or(DownloadError::InvalidRedirect)?;
            let next = current
                .url
                .resolve(location)
                .map_err(|_| DownloadError::InvalidRedirect)?;
            if !matches!(next.scheme(), "http" | "https") {
                return Err(DownloadError::InvalidRedirect);
            }
            if current.url.is_secure() && next.scheme() == "http" {
                return Err(DownloadError::SecureDowngradeBlocked);
            }
            if current.url.scheme() != next.scheme() || current.url.host() != next.host() {
                current.headers.retain(|name, _| {
                    !name.eq_ignore_ascii_case("authorization")
                        && !name.eq_ignore_ascii_case("cookie")
                });
            }
            current.url = next;
            continue;
        }

        if !(200..300).contains(&response.status) {
            return Err(DownloadError::HttpStatus(response.status));
        }
        if response.body.len() > max_bytes {
            return Err(DownloadError::ResponseTooLarge);
        }

        let suggested_filename = response
            .header("content-disposition")
            .and_then(filename_from_content_disposition)
            .or_else(|| filename_from_url(&current.url))
            .unwrap_or_else(|| "download.bin".to_owned());
        publish_atomically(destination, &response.body)?;

        return Ok(DownloadResult {
            final_url: current.url,
            destination: destination.to_path_buf(),
            suggested_filename,
            content_type: response.header("content-type").map(str::to_owned),
            bytes_written: response.body.len(),
        });
    }

    Err(DownloadError::RedirectLimitExceeded)
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn filename_from_content_disposition(value: &str) -> Option<String> {
    let extended = value
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename*="))
        .and_then(|value| value.split_once("''").map(|(_, value)| value));
    let plain = value
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix("filename="))
        .map(|value| value.trim_matches('"'));
    extended.or(plain).and_then(sanitize_filename)
}

fn filename_from_url(url: &Url) -> Option<String> {
    url.path()
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .and_then(sanitize_filename)
}

fn sanitize_filename(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == "." || value == ".." {
        return None;
    }
    let mut safe = value
        .chars()
        .map(|character| {
            if character.is_control()
                || matches!(
                    character,
                    '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
                )
            {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    while safe.ends_with([' ', '.']) {
        safe.pop();
    }
    if safe.is_empty() {
        return None;
    }
    let stem = safe.split('.').next().unwrap_or_default();
    if matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        safe.insert(0, '_');
    }
    Some(safe)
}

fn publish_atomically(destination: &Path, body: &[u8]) -> Result<(), DownloadError> {
    if let Some(parent) = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }

    let file_name = destination
        .file_name()
        .ok_or_else(|| DownloadError::Io("destination has no filename".into()))?;
    let mut temporary_path = None;
    let mut temporary_file = None;
    for attempt in 0..32_u32 {
        let mut temporary_name = file_name.to_os_string();
        temporary_name.push(format!(".archia-{}-{attempt}.part", std::process::id()));
        let candidate = destination.with_file_name(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary_path = Some(candidate);
                temporary_file = Some(file);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
    let temporary_path = temporary_path
        .ok_or_else(|| DownloadError::Io("could not reserve a temporary download file".into()))?;
    let mut file = temporary_file.expect("temporary path and file are created together");
    let write_result = file.write_all(body).and_then(|_| file.sync_all());
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(io_error(error));
    }
    if destination.exists() {
        let _ = fs::remove_file(&temporary_path);
        return Err(DownloadError::DestinationExists);
    }
    if let Err(error) = fs::rename(&temporary_path, destination) {
        let _ = fs::remove_file(&temporary_path);
        return Err(io_error(error));
    }
    Ok(())
}

fn io_error(error: std::io::Error) -> DownloadError {
    DownloadError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Response, Transport};
    use std::sync::Mutex;

    struct QueueTransport(Mutex<Vec<Response>>);

    impl Transport for QueueTransport {
        fn send(&self, _request: &Request) -> Result<Response, TransportError> {
            let mut responses = self.0.lock().unwrap();
            if responses.is_empty() {
                return Err(TransportError::ConnectionFailed);
            }
            Ok(responses.remove(0))
        }
    }

    fn destination(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "archia-browser-download-{}-{label}.bin",
            std::process::id()
        ))
    }

    #[test]
    fn saves_body_and_uses_content_disposition_filename() {
        let path = destination("disposition");
        let _ = fs::remove_file(&path);
        let transport = QueueTransport(Mutex::new(vec![Response::new(200)
            .with_header("content-type", "application/octet-stream")
            .with_header("content-disposition", "attachment; filename=\"report.bin\"")
            .with_body(b"download payload".to_vec())]));
        let request = Request::new(Url::parse("https://example.org/file").unwrap());
        let result = download_to_path(&transport, &request, &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"download payload");
        assert_eq!(result.suggested_filename, "report.bin");
        assert_eq!(result.bytes_written, 16);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn follows_redirects_before_publishing() {
        let path = destination("redirect");
        let _ = fs::remove_file(&path);
        let transport = QueueTransport(Mutex::new(vec![
            Response::new(302).with_header("location", "/asset.bin"),
            Response::new(200).with_body(b"ok".to_vec()),
        ]));
        let request = Request::new(Url::parse("https://example.org/start").unwrap());
        let result = download_to_path(&transport, &request, &path).unwrap();
        assert_eq!(
            result.final_url.to_string(),
            "https://example.org/asset.bin"
        );
        assert_eq!(fs::read(&path).unwrap(), b"ok");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn blocks_https_downgrade_and_oversized_bodies() {
        let path = destination("blocked");
        let _ = fs::remove_file(&path);
        let redirect = QueueTransport(Mutex::new(vec![
            Response::new(302).with_header("location", "http://example.org/file")
        ]));
        let request = Request::new(Url::parse("https://example.org/start").unwrap());
        assert_eq!(
            download_to_path(&redirect, &request, &path),
            Err(DownloadError::SecureDowngradeBlocked)
        );

        let oversized = QueueTransport(Mutex::new(vec![Response::new(200).with_body(vec![0; 8])]));
        assert_eq!(
            download_to_path_with_limit(&oversized, &request, &path, 4),
            Err(DownloadError::ResponseTooLarge)
        );
        assert!(!path.exists());
    }

    #[test]
    fn sanitizes_untrusted_filenames() {
        assert_eq!(
            sanitize_filename("../CON.txt").as_deref(),
            Some(".._CON.txt")
        );
        assert_eq!(sanitize_filename("   "), None);
        assert_eq!(
            sanitize_filename("folder/file.exe").as_deref(),
            Some("folder_file.exe")
        );
    }
}
