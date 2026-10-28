//! Opens files in the running viewer.
//!
//! A launch with only files hands them to the first instance and exits. The first instance
//! listens on a local socket. A later launch sends it the absolute path of each file, each
//! followed by a zero byte, and another one at the end. The first instance answers with one
//! zero byte once it has the files.

use futures::channel::mpsc::UnboundedSender;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;

pub enum InstanceRole {
    Forwarded,
    Primary(LocalSocketListener),
}

pub fn socket_name() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let build_suffix = if cfg!(debug_assertions) { "-debug" } else { "" };
    format!("illuminator{build_suffix}-{user}.sock")
}

pub fn claim_primary_instance(name: &str, files: &[PathBuf]) -> io::Result<InstanceRole> {
    let lock_file = File::create(std::env::temp_dir().join(format!("{name}.lock")))?;
    lock_file.lock()?;
    let name = name.to_ns_name::<GenericNamespaced>()?;
    if let Ok(stream) = LocalSocketStream::connect(name.borrow()) {
        send_paths(stream, files)?;
        return Ok(InstanceRole::Forwarded);
    }
    let listener = ListenerOptions::new()
        .name(name)
        // if nobody listens, this should be a socket file left behind by a crash
        .try_overwrite(true)
        .create_sync()?;
    Ok(InstanceRole::Primary(listener))
}

pub fn listen_for_open_requests(
    listener: LocalSocketListener,
    open_request_sender: UnboundedSender<Vec<PathBuf>>,
) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream.and_then(receive_paths) {
                Ok(files) => {
                    if open_request_sender.unbounded_send(files).is_err() {
                        break; // viewer is gone
                    }
                }
                Err(error) => eprintln!("Illuminator: {error}"),
            }
        }
    });
}

fn send_paths(mut stream: LocalSocketStream, files: &[PathBuf]) -> io::Result<()> {
    let mut message = Vec::new();
    for file in files {
        let path = std::path::absolute(file)?;
        let Some(path) = path.to_str() else {
            eprintln!("Illuminator: cannot hand over {}", path.display());
            continue;
        };
        message.extend_from_slice(path.as_bytes());
        message.push(0);
    }
    message.push(0);
    // Windows only lets the foreground app bring a window to the front, so allow the first instance to show
    // its window when it receives the new files.
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
        // SAFETY: AllowSetForegroundWindow takes no pointers. Failing should be harmless.
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    }
    stream.write_all(&message)?;
    stream.read_exact(&mut [0])
}

fn receive_paths(stream: LocalSocketStream) -> io::Result<Vec<PathBuf>> {
    let mut stream = BufReader::new(stream);
    let mut files = Vec::new();
    loop {
        let mut path = Vec::new();
        stream.read_until(0, &mut path)?;
        if path.pop() != Some(0) {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if path.is_empty() {
            break;
        }
        let path = String::from_utf8(path)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        files.push(PathBuf::from(path));
    }
    stream.get_mut().write_all(&[0])?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::{InstanceRole, claim_primary_instance, listen_for_open_requests};
    use futures::StreamExt;
    use futures::channel::mpsc;
    use futures::executor::block_on;
    use std::path::PathBuf;

    #[test]
    fn a_second_instance_hands_its_files_to_the_first() {
        let name = format!("illuminator-test-{}.sock", std::process::id());
        let InstanceRole::Primary(listener) = claim_primary_instance(&name, &[]).unwrap() else {
            panic!("nobody listens yet");
        };
        let (open_request_sender, mut received) = mpsc::unbounded();
        listen_for_open_requests(listener, open_request_sender);

        let cwd = std::env::current_dir().unwrap();
        let files = [PathBuf::from("ct.nii.gz"), cwd.join("scans/my pet.nii")];
        assert!(matches!(
            claim_primary_instance(&name, &files),
            Ok(InstanceRole::Forwarded)
        ));
        assert_eq!(
            block_on(received.next()),
            Some(vec![cwd.join("ct.nii.gz"), cwd.join("scans/my pet.nii")])
        );
        assert!(matches!(
            claim_primary_instance(&name, &[]),
            Ok(InstanceRole::Forwarded)
        ));
        assert_eq!(block_on(received.next()), Some(vec![]));
    }
}
