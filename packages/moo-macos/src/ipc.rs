//! The CLI's local connection: a Unix socket on macOS, a named pipe on Windows, behind the few
//! calls cli.rs makes (`connect`, `bind`, `incoming`, `try_clone`, timeouts, Read and Write).

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

#[cfg(windows)]
pub use pipe::{Listener, Stream};

#[cfg(windows)]
mod pipe {
    use std::io::{self, Read, Write};
    use std::path::Path;
    use std::time::Duration;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::*;
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::Pipes::*;
    use windows::Win32::System::Threading::GetCurrentProcess;

    fn wide(p: &Path) -> Vec<u16> {
        p.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// One end of a named-pipe connection.
    pub struct Stream(HANDLE);

    unsafe impl Send for Stream {}
    unsafe impl Sync for Stream {}

    impl Drop for Stream {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    impl Stream {
        pub fn connect(path: impl AsRef<Path>) -> io::Result<Stream> {
            let name = wide(path.as_ref());
            unsafe {
                let h = CreateFileW(PCWSTR(name.as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_NONE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None)
                    .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
                Ok(Stream(h))
            }
        }

        pub fn try_clone(&self) -> io::Result<Stream> {
            let mut dup = HANDLE::default();
            unsafe {
                DuplicateHandle(GetCurrentProcess(), self.0, GetCurrentProcess(), &mut dup, 0, false, DUPLICATE_SAME_ACCESS)
                    .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            }
            Ok(Stream(dup))
        }

        /// Byte-mode pipes have no per-call timeouts; a stuck peer is cut off when it closes.
        pub fn set_read_timeout(&self, _t: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        pub fn set_write_timeout(&self, _t: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut n = 0u32;
            match unsafe { ReadFile(self.0, Some(buf), Some(&mut n), None) } {
                Ok(()) => Ok(n as usize),
                // The other end closed: end of stream.
                Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() => Ok(0),
                Err(e) => Err(io::Error::from_raw_os_error(e.code().0)),
            }
        }
    }

    impl Read for &Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let mut n = 0u32;
            match unsafe { ReadFile(self.0, Some(buf), Some(&mut n), None) } {
                Ok(()) => Ok(n as usize),
                Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() => Ok(0),
                Err(e) => Err(io::Error::from_raw_os_error(e.code().0)),
            }
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let mut n = 0u32;
            unsafe { WriteFile(self.0, Some(buf), Some(&mut n), None) }.map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            Ok(n as usize)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// The server end: each `incoming()` item is one connected client.
    pub struct Listener(Vec<u16>);

    impl Listener {
        pub fn bind(path: impl AsRef<Path>) -> io::Result<Listener> {
            let l = Listener(wide(path.as_ref()));
            // Fail now (like a socket bind) if the name can't be created, e.g. another app owns it.
            let probe = l.instance(true)?;
            unsafe {
                let _ = CloseHandle(probe);
            }
            Ok(l)
        }

        fn instance(&self, first: bool) -> io::Result<HANDLE> {
            let mode = if first { PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_DUPLEX };
            unsafe {
                let h = CreateNamedPipeW(
                    PCWSTR(self.0.as_ptr()),
                    mode,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    PIPE_UNLIMITED_INSTANCES,
                    64 * 1024,
                    64 * 1024,
                    0,
                    None,
                );
                if h.is_invalid() {
                    return Err(io::Error::last_os_error());
                }
                Ok(h)
            }
        }

        pub fn incoming(&self) -> impl Iterator<Item = io::Result<Stream>> + '_ {
            std::iter::from_fn(move || {
                let h = match self.instance(false) {
                    Ok(h) => h,
                    Err(e) => return Some(Err(e)),
                };
                match unsafe { ConnectNamedPipe(h, None) } {
                    Ok(()) => Some(Ok(Stream(h))),
                    // The client connected between create and connect: still a connection.
                    Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Some(Ok(Stream(h))),
                    Err(e) => {
                        unsafe {
                            let _ = CloseHandle(h);
                        }
                        Some(Err(io::Error::from_raw_os_error(e.code().0)))
                    }
                }
            })
        }
    }
}
