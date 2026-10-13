// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: MIT

//! The host link of a desktop simulator: a pseudo-terminal, which a program that drives a
//! device over a serial port opens like one.

extern crate std;

use alloc::string::String;

/// Creates the pseudo-terminal and prints the name of its device.
/// Slint must already use a desktop backend, so nothing else needs initializing.
pub fn init_with_host_link() -> impl crate::HostLink {
    let link = imp::PseudoTerminal::open().expect("Cannot create a pseudo-terminal");
    std::println!("The simulator's serial port is {}", link.name());
    link
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::time::{Duration, Instant};

    pub struct PseudoTerminal {
        main: std::fs::File,
        name: String,
        /// Held open, so that the main side keeps working while no program has the port open.
        _subsidiary: OwnedFd,
    }

    impl PseudoTerminal {
        pub fn open() -> std::io::Result<Self> {
            let check = |result: libc::c_int| {
                if result < 0 { Err(std::io::Error::last_os_error()) } else { Ok(result) }
            };
            // SAFETY: plain libc calls on a descriptor this function owns
            unsafe {
                let main = check(libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY))?;
                let main = OwnedFd::from_raw_fd(main);
                check(libc::grantpt(main.as_raw_fd()))?;
                check(libc::unlockpt(main.as_raw_fd()))?;
                let name = libc::ptsname(main.as_raw_fd());
                if name.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let name = std::ffi::CStr::from_ptr(name).to_string_lossy().into_owned();
                let subsidiary = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(libc::O_NOCTTY)
                    .open(&name)?;
                // Bytes pass through unchanged, as on a serial port
                let mut termios = core::mem::zeroed::<libc::termios>();
                check(libc::tcgetattr(subsidiary.as_raw_fd(), &mut termios))?;
                libc::cfmakeraw(&mut termios);
                check(libc::tcsetattr(subsidiary.as_raw_fd(), libc::TCSANOW, &termios))?;
                let flags = check(libc::fcntl(main.as_raw_fd(), libc::F_GETFL))?;
                check(libc::fcntl(main.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK))?;
                Ok(Self { main: main.into(), name, _subsidiary: subsidiary.into() })
            }
        }

        pub fn name(&self) -> &str {
            &self.name
        }
    }

    use std::os::unix::fs::OpenOptionsExt as _;

    impl crate::HostLink for PseudoTerminal {
        fn read(&mut self, buffer: &mut [u8]) -> usize {
            self.main.read(buffer).unwrap_or(0)
        }

        fn write(&mut self, mut bytes: &[u8]) {
            // Nobody may be reading: give up on what doesn't go out within a moment
            let deadline = Instant::now() + Duration::from_millis(100);
            while !bytes.is_empty() && Instant::now() < deadline {
                match self.main.write(bytes) {
                    Ok(written) => bytes = &bytes[written..],
                    Err(_) => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        }

        fn board_name(&self) -> &str {
            "Simulator"
        }

        fn port_name(&self) -> Option<&str> {
            Some(&self.name)
        }

        fn reset(&mut self) {}
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;

    /// Windows has no pseudo-terminal that a serial port library opens: this one never
    /// receives anything.
    pub struct PseudoTerminal;

    impl PseudoTerminal {
        pub fn open() -> std::io::Result<Self> {
            Ok(Self)
        }

        pub fn name(&self) -> &str {
            "unavailable: the simulator only has one on Linux and macOS"
        }
    }

    impl crate::HostLink for PseudoTerminal {
        fn read(&mut self, _buffer: &mut [u8]) -> usize {
            0
        }

        fn write(&mut self, _bytes: &[u8]) {}

        fn board_name(&self) -> &str {
            "Simulator"
        }

        fn reset(&mut self) {}
    }
}
