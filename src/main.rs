use libc::{c_int, termios as Termios};
use libc::{BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST};
use std::io::{self, Error, ErrorKind, Read, Result};
use std::mem;

extern "C" {
    pub fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
    pub fn tcsetattr(fd: c_int, optional_actions: c_int, termios: *const Termios) -> c_int;
    pub fn iscntrl(c: c_int) -> c_int;
}

trait TermiosAttrExt {
    fn get_attr(&mut self) -> Result<()>;
    fn set_attr(&self) -> Result<()>;
}

impl TermiosAttrExt for Termios {
    fn get_attr(&mut self) -> Result<()> {
        Ok(unsafe {
            if tcgetattr(libc::STDIN_FILENO, self) != 0 {
                return Err(Error::new(ErrorKind::Other, "Can't get term attributes"));
            }
        })
    }

    fn set_attr(&self) -> Result<()> {
        Ok(unsafe {
            if tcsetattr(libc::STDIN_FILENO, libc::TCSAFLUSH, self) != 0 {
                return Err(Error::new(ErrorKind::Other, "Can't get term attributes"));
            }
        })
    }
}

struct Terminal {
    orig_flags: Termios,
    curr_flags: Termios,
}

impl Terminal {
    pub fn new() -> Result<Self> {
        let mut orig_flags = unsafe { mem::zeroed::<Termios>() };
        orig_flags.get_attr()?;
        Ok(Self {
            orig_flags,
            curr_flags: orig_flags.clone(),
        })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.orig_flags
            .set_attr()
            .expect("Failed to restore terminal state");
    }
}

fn raw_mode_terminal() -> Result<Terminal> {
    let mut terminal = Terminal::new()?;
    terminal.curr_flags.c_lflag &= !(ECHO | ICANON | ISIG | IEXTEN);
    terminal.curr_flags.c_iflag &= !(IXON | ICRNL | BRKINT | INPCK | ISTRIP);
    terminal.curr_flags.c_oflag &= !(OPOST);
    terminal.curr_flags.c_oflag |= CS8;
    terminal.curr_flags.set_attr()?;
    Ok(terminal)
}

fn ctrl_key(byte: u8) -> u8 {
    byte & 0x1F
}

fn main() -> Result<()> {
    let _terminal = raw_mode_terminal()?;

    for byte in io::stdin().bytes() {
        let byte = byte.unwrap_or(b'q');
        if unsafe { iscntrl(byte as i32) != 0 } {
            print!("{:02x}\r\n", byte as u8);
        } else {
            print!("{:02x} ({})\r\n", byte as u8, byte as char);
        }
        if byte == ctrl_key(b'q') {
            return Ok(());
        }
    }
    Err(Error::new(ErrorKind::Other, "Reading from stream failed"))
}
