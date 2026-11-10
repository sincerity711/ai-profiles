//! Persistent, release-bound archive downloads. The caller holds the app lock.
use super::{feed, read_json, write_json, AppKind, Release, Result, MAX_ARCHIVE};
use reqwest::{blocking::Client, header, StatusCode};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn prepare(dir: &Path, release: &Release) -> Result<PathBuf> {
    if release.size.is_some_and(|size| size > MAX_ARCHIVE) {
        return Err("Update archive exceeds the 2 GB download limit".into());
    }
    let record = dir.join("release.json");
    // Never append bytes from another build, URL, checksum or signature.
    if read_json::<Release>(&record)?.as_ref() != Some(release) {
        if dir.exists() {
            fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        write_json(&record, release)?;
    }
    Ok(dir.join("update.zip"))
}

pub(super) fn download_verified(
    app: AppKind,
    client: &Client,
    release: &Release,
    dir: &Path,
    mut progress: impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<PathBuf> {
    let archive = prepare(dir, release)?;
    transfer(client, release, &archive, &mut progress)?;
    let bytes = fs::read(&archive).map_err(|e| e.to_string())?;
    if let Err(error) = feed::verify_archive(app, release, &bytes) {
        // A complete but corrupt archive must not poison every later retry.
        fs::remove_file(&archive).map_err(|e| e.to_string())?;
        progress(0, release.size)?;
        return Err(error);
    }
    Ok(archive)
}

fn range_total(response: &reqwest::blocking::Response, offset: u64) -> Result<u64> {
    let invalid = || "Invalid update Content-Range; retry the download".to_string();
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(invalid)?;
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|v| v.split_once('/'))
        .ok_or_else(invalid)?;
    let (start, end) = range.split_once('-').ok_or_else(invalid)?;
    let start: u64 = start.parse().map_err(|_| invalid())?;
    let end: u64 = end.parse().map_err(|_| invalid())?;
    let total: u64 = total.parse().map_err(|_| invalid())?;
    if start != offset
        || start > end
        || total == 0
        || end != total - 1
        || response
            .content_length()
            .is_some_and(|n| n != total - start)
    {
        return Err(invalid());
    }
    Ok(total)
}

fn transfer(
    client: &Client,
    release: &Release,
    archive: &Path,
    progress: &mut impl FnMut(u64, Option<u64>) -> Result<()>,
) -> Result<()> {
    let mut offset = match fs::metadata(archive) {
        Ok(meta) => meta.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.to_string()),
    };
    if offset > MAX_ARCHIVE || release.size.is_some_and(|size| offset > size) {
        fs::remove_file(archive).map_err(|e| e.to_string())?;
        offset = 0;
    }
    progress(offset, release.size)?;
    if offset > 0 && release.size == Some(offset) {
        // A crash after the final write needs only verification, not another GET.
        return Ok(());
    }
    let request = |offset| {
        let mut request = client
            .get(&release.url)
            .header(header::ACCEPT_ENCODING, "identity");
        if offset > 0 {
            request = request.header(header::RANGE, format!("bytes={offset}-"));
        }
        request.send().map_err(|e| e.to_string())
    };
    let mut response = request(offset)?;
    if offset > 0 && response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        // A server may refuse ranges; retry once with a full request.
        // Keep the old partial file until the new response has been validated.
        response = request(0)?;
        offset = 0;
    }
    response = response.error_for_status().map_err(|e| e.to_string())?;
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v.as_bytes() != b"identity")
    {
        return Err("Unexpected encoded update archive".into());
    }
    let total = match response.status() {
        StatusCode::PARTIAL_CONTENT => Some(range_total(&response, offset)?),
        StatusCode::OK => {
            // Servers ignoring Range return the entire archive: replace, never append.
            offset = 0;
            response.content_length()
        }
        status => return Err(format!("Unexpected update download status: {status}")),
    };
    if total.is_some_and(|size| size > MAX_ARCHIVE)
        || release
            .size
            .zip(total)
            .is_some_and(|(expected, actual)| expected != actual)
    {
        return Err("Unexpected update archive size".into());
    }
    let total = release.size.or(total);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(offset == 0)
        .append(offset > 0)
        .open(archive)
        .map_err(|e| e.to_string())?;
    progress(offset, total)?;
    let mut downloaded = offset;
    let mut buffer = vec![0; 256 * 1024];
    let mut last = Instant::now();
    let result = (|| {
        loop {
            let n = response.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            if downloaded + n as u64 > total.unwrap_or(MAX_ARCHIVE) {
                return Err("Update archive exceeds the expected download size".into());
            }
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            downloaded += n as u64;
            if last.elapsed() > Duration::from_millis(400) {
                progress(downloaded, total)?;
                last = Instant::now();
            }
        }
        if total.is_some_and(|size| downloaded != size) {
            return Err("Download interrupted; retry to resume".into());
        }
        Ok(())
    })();
    // Persist and report the actual file length even when a read/write failed.
    file.sync_all().map_err(|e| e.to_string())?;
    progress(file.metadata().map_err(|e| e.to_string())?.len(), total)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
        thread,
    };

    fn client() -> Client {
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }

    fn release(url: String) -> Release {
        Release {
            version: "1.0".into(),
            build: "1".into(),
            url,
            size: Some(6),
            sha256: Some(format!("{:x}", Sha256::digest(b"abcdef"))),
            signature: None,
        }
    }

    fn server(responses: Vec<&'static str>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/update.zip", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for response in responses {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "Missing download request");
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                requests.push(request.to_lowercase());
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (url, handle)
    }

    const FULL: &str = "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef";
    const PARTIAL: &str = "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\nConnection: close\r\n\r\ndef";

    #[test]
    fn interrupted_download_resumes_from_disk_and_reports_full_progress() {
        let (url, server) = server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc",
            PARTIAL,
        ]);
        let release = release(url);
        let dir = tempfile::tempdir().unwrap();
        let mut progress = Vec::new();
        assert!(download_verified(
            AppKind::Claude,
            &client(),
            &release,
            dir.path(),
            |n, total| {
                progress.push((n, total));
                Ok(())
            }
        )
        .is_err());
        assert_eq!(fs::read(dir.path().join("update.zip")).unwrap(), b"abc");
        assert_eq!(progress.last(), Some(&(3, Some(6))));
        progress.clear();
        let archive = download_verified(
            AppKind::Claude,
            &client(),
            &release,
            dir.path(),
            |n, total| {
                progress.push((n, total));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(fs::read(archive).unwrap(), b"abcdef");
        assert_eq!(progress.first(), Some(&(3, Some(6))));
        assert_eq!(progress.last(), Some(&(6, Some(6))));
        let requests = server.join().unwrap();
        assert!(!requests[0].contains("range:"));
        assert!(requests[1].contains("range: bytes=3-"));
        assert!(requests[1].contains("accept-encoding: identity"));
    }

    #[test]
    fn ignored_or_rejected_ranges_restart_without_appending_full_response() {
        for responses in [vec![FULL], vec![
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", FULL,
        ]] {
            let (url, server) = server(responses);
            let release = release(url);
            let dir = tempfile::tempdir().unwrap();
            fs::write(prepare(dir.path(), &release).unwrap(), b"abc").unwrap();
            let archive = download_verified(AppKind::Claude, &client(), &release, dir.path(), |_, _| Ok(())).unwrap();
            assert_eq!(fs::read(archive).unwrap(), b"abcdef");
            let requests = server.join().unwrap();
            assert!(requests[0].contains("range: bytes=3-"));
            if requests.len() == 2 { assert!(!requests[1].contains("range:")); }
        }
    }

    #[test]
    fn invalid_responses_never_modify_the_partial_archive() {
        for response in [
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 2-4/6\r\nConnection: close\r\n\r\ncde",
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nConnection: close\r\n\r\ndef",
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-6/7\r\nConnection: close\r\n\r\ndef",
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 3-6/7\r\nConnection: close\r\n\r\ndefg",
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nabcdefg",
            "HTTP/1.1 200 OK\r\nContent-Length: 2147483649\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Encoding: compress\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef",
        ] {
            let (url, server) = server(vec![response]);
            let release = release(url);
            let dir = tempfile::tempdir().unwrap();
            let archive = prepare(dir.path(), &release).unwrap();
            fs::write(&archive, b"abc").unwrap();
            assert!(download_verified(AppKind::Claude, &client(), &release, dir.path(), |_, _| Ok(())).is_err());
            assert_eq!(fs::read(archive).unwrap(), b"abc");
            server.join().unwrap();
        }
    }

    #[test]
    fn changed_release_discards_old_bytes_before_requesting() {
        let (url, server) = server(vec![FULL]);
        let release = release(url);
        let dir = tempfile::tempdir().unwrap();
        let mut old = release.clone();
        old.sha256 = Some("0".repeat(64));
        fs::write(prepare(dir.path(), &old).unwrap(), b"old").unwrap();
        let archive =
            download_verified(AppKind::Claude, &client(), &release, dir.path(), |_, _| {
                Ok(())
            })
            .unwrap();
        assert_eq!(fs::read(archive).unwrap(), b"abcdef");
        assert!(!server.join().unwrap()[0].contains("range:"));
    }

    #[test]
    fn completed_archive_is_reverified_offline_and_corruption_is_discarded() {
        let release = release("http://127.0.0.1:1/unreachable".into());
        let dir = tempfile::tempdir().unwrap();
        let archive = prepare(dir.path(), &release).unwrap();
        fs::write(&archive, b"abcdef").unwrap();
        download_verified(AppKind::Claude, &client(), &release, dir.path(), |_, _| {
            Ok(())
        })
        .unwrap();
        fs::write(&archive, b"broken").unwrap();
        let mut progress = Vec::new();
        let error = download_verified(AppKind::Claude, &client(), &release, dir.path(), |n, _| {
            progress.push(n);
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("checksum mismatch"));
        assert!(!archive.exists());
        assert_eq!(progress.last(), Some(&0));
    }

    #[test]
    fn missing_feed_size_uses_full_content_range_total() {
        let (url, server) = server(vec![PARTIAL]);
        let mut release = release(url);
        release.size = None;
        let dir = tempfile::tempdir().unwrap();
        fs::write(prepare(dir.path(), &release).unwrap(), b"abc").unwrap();
        let mut progress = Vec::new();
        download_verified(
            AppKind::Claude,
            &client(),
            &release,
            dir.path(),
            |n, total| {
                progress.push((n, total));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(progress.last(), Some(&(6, Some(6))));
        server.join().unwrap();
    }
}
