use futures::channel::mpsc::UnboundedSender;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;

pub enum Instance {
    Forwarded,
    First(LocalSocketListener),
}

pub fn socket_name() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let build = if cfg!(debug_assertions) { "-debug" } else { "" };
    format!("illuminator{build}-{user}.sock")
}

pub fn claim(name: &str, files: &[PathBuf]) -> io::Result<Instance> {
    let lock = File::create(std::env::temp_dir().join(format!("{name}.lock")))?;
    lock.lock()?;
    let name = name.to_ns_name::<GenericNamespaced>()?;
    if let Ok(stream) = LocalSocketStream::connect(name.borrow()) {
        send(stream, files)?;
        return Ok(Instance::Forwarded);
    }
    let listener = ListenerOptions::new()
        .name(name)
        // if nobody listens, this should be a socket file left behind by a crash
        .try_overwrite(true)
        .create_sync()?;
    Ok(Instance::First(listener))
}

pub fn listen(listener: LocalSocketListener, opened: UnboundedSender<Vec<PathBuf>>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream.and_then(receive) {
                Ok(files) => {
                    if opened.unbounded_send(files).is_err() {
                        break; // viewer is gone
                    }
                }
                Err(error) => eprintln!("Illuminator: {error}"),
            }
        }
    });
}

fn send(mut stream: LocalSocketStream, files: &[PathBuf]) -> io::Result<()> {
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

fn receive(stream: LocalSocketStream) -> io::Result<Vec<PathBuf>> {
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
    use super::{Instance, claim, listen};
    use futures::StreamExt;
    use futures::channel::mpsc;
    use futures::executor::block_on;
    use std::path::PathBuf;

    #[test]
    fn a_second_instance_hands_its_files_to_the_first() {
        let name = format!("illuminator-test-{}.sock", std::process::id());
        let Instance::First(listener) = claim(&name, &[]).unwrap() else {
            panic!("nobody listens yet");
        };
        let (opened, mut received) = mpsc::unbounded();
        listen(listener, opened);

        let cwd = std::env::current_dir().unwrap();
        let files = [PathBuf::from("ct.nii.gz"), cwd.join("scans/my pet.nii")];
        assert!(matches!(claim(&name, &files), Ok(Instance::Forwarded)));
        assert_eq!(
            block_on(received.next()),
            Some(vec![cwd.join("ct.nii.gz"), cwd.join("scans/my pet.nii")])
        );
        assert!(matches!(claim(&name, &[]), Ok(Instance::Forwarded)));
        assert_eq!(block_on(received.next()), Some(vec![]));
    }
}
