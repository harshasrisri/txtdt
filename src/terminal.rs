use libc::{c_int, c_ulong, c_void, termios as Termios, winsize as WinSize};
use libc::{
    BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, STDIN_FILENO,
    STDOUT_FILENO, TIOCGWINSZ, VMIN, VTIME,
};
use std::io::{self, Error, ErrorKind, Read, Result};
use std::mem;

extern "C" {
    pub fn tcgetattr(fd: c_int, termios: *mut Termios) -> c_int;
    pub fn tcsetattr(fd: c_int, optional_actions: c_int, termios: *const Termios) -> c_int;
    pub fn iscntrl(c: c_int) -> c_int;
    pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
}

trait TermiosAttrExt {
    fn get_attr() -> Result<Termios>;
    fn set_attr(&self) -> Result<()>;
    fn enable_raw_mode(&mut self) -> Result<()>;
}

impl TermiosAttrExt for Termios {
    fn get_attr() -> Result<Termios> {
        let mut termios = unsafe { mem::zeroed::<Termios>() };
        unsafe {
            if tcgetattr(STDIN_FILENO, &mut termios) != 0 {
                return Err(Error::new(ErrorKind::Other, "Can't get term attributes"));
            }
        }
        Ok(termios)
    }

    fn set_attr(&self) -> Result<()> {
        unsafe {
            if tcsetattr(STDIN_FILENO, libc::TCSAFLUSH, self) != 0 {
                return Err(Error::new(ErrorKind::Other, "Can't get term attributes"));
            }
        }
        Ok(())
    }

    fn enable_raw_mode(&mut self) -> Result<()> {
        self.c_lflag &= !(ECHO | ICANON | ISIG | IEXTEN);
        self.c_iflag &= !(IXON | ICRNL | BRKINT | INPCK | ISTRIP);
        self.c_oflag &= !(OPOST);
        self.c_oflag |= CS8;
        self.c_cc[VMIN] = 0;
        self.c_cc[VTIME] = 1;
        self.set_attr()
    }
}

trait WinSizeAttrExt {
    fn get_window_size() -> Result<(usize, usize)>;
    fn get_cursor_position() -> Result<(usize, usize)>;
}

impl WinSizeAttrExt for WinSize {
    fn get_window_size() -> Result<(usize, usize)> {
        let mut ws = unsafe { mem::zeroed::<WinSize>() };
        unsafe {
            if ioctl(STDOUT_FILENO, TIOCGWINSZ, &mut ws) == -1 || ws.ws_col == 0 {
                let botright = "\x1b[999C\x1b[999B";
                if Terminal::write(botright) != botright.len() as isize {
                    return Err(Error::new(ErrorKind::Other, "Can't get window size"));
                }
                Self::get_cursor_position()?;
            }
            Ok((ws.ws_row as usize, ws.ws_col as usize))
        }
    }

    fn get_cursor_position() -> Result<(usize, usize)> {
        Terminal::write("\x1b[6n");
        print!("\r\n");

        let cursor_buf = io::stdin()
            .bytes()
            .take_while(|c| matches!(c, Ok(b'R')))
            .collect::<Result<Vec<_>>>()?;

        let dimensions = cursor_buf[2..]
            .split(|&c| c == b';')
            .filter_map(|buf| std::str::from_utf8(buf).ok())
            .filter_map(|buf| buf.parse().ok())
            .collect::<Vec<_>>();

        if dimensions.len() != 2 {
            return Err(Error::new(ErrorKind::Other, "Can't get window size"));
        }

        Ok((dimensions[0], dimensions[1]))
    }
}

pub enum Motion {
    Up,
    Down,
    Left,
    Right,
    PgUp,
    PgDn,
    Home,
    End,
}

pub enum Key {
    Printable(u8),
    Move(Motion),
    Control(char),
    Delete,
    Backspace,
    Newline,
    Escape,
}

pub struct Terminal {
    orig_termios: Termios,
    curr_termios: Termios,
    num_rows: usize,
    num_cols: usize,
    term_buffer: String,
    key_buffer: Vec<u8>,
}

impl Terminal {
    pub fn new() -> Result<Self> {
        let (num_rows, num_cols) = WinSize::get_window_size()?;
        let orig_termios = Termios::get_attr()?;
        Ok(Self {
            orig_termios,
            curr_termios: orig_termios,
            num_rows,
            num_cols,
            term_buffer: String::new(),
            key_buffer: Vec::new(),
        })
    }

    pub fn make_raw(&mut self) -> Result<()> {
        self.curr_termios.enable_raw_mode()
    }

    pub fn new_raw() -> Result<Self> {
        let mut term = Terminal::new()?;
        term.make_raw()?;
        Ok(term)
    }

    pub fn rows(&self) -> usize {
        self.num_rows
    }

    pub fn cols(&self) -> usize {
        self.num_cols
    }

    pub fn write(seq: &str) -> isize {
        unsafe { libc::write(STDOUT_FILENO, seq.as_ptr() as *const c_void, seq.len()) }
    }

    pub fn append(&mut self, content: &str) {
        self.term_buffer.push_str(content);
    }

    pub fn flush(&mut self) {
        Terminal::write(self.term_buffer.as_str());
        self.term_buffer.clear();
    }

    pub fn read_key(&mut self) -> Result<Key> {
        let read_key = || io::stdin().bytes().next();
        let key = if let Some(pending_key) = self.key_buffer.pop() {
            pending_key
        } else {
            std::iter::repeat_with(read_key)
                .skip_while(|c| c.is_none())
                .flatten()
                .next()
                .unwrap()?
        };

        Ok(if key == b'\x1b' {
            let seq = self
                .key_buffer
                .iter()
                .rev()
                .map(|byte| Some(Ok(*byte)))
                .chain(std::iter::repeat_with(read_key))
                .take(3)
                .map(|k| k.transpose())
                .collect::<Result<Vec<Option<u8>>>>()?;

            let (key, pending) = match seq.as_slice() {
                [None, None, None] => (Key::Escape, None),

                [Some(b'['), Some(b'A'), pending] => (Key::Move(Motion::Up), *pending),
                [Some(b'['), Some(b'B'), pending] => (Key::Move(Motion::Down), *pending),
                [Some(b'['), Some(b'C'), pending] => (Key::Move(Motion::Right), *pending),
                [Some(b'['), Some(b'D'), pending] => (Key::Move(Motion::Left), *pending),

                [Some(b'['), Some(b'5'), Some(b'~')] => (Key::Move(Motion::PgUp), None),
                [Some(b'['), Some(b'6'), Some(b'~')] => (Key::Move(Motion::PgDn), None),

                [Some(b'['), Some(b'1'), Some(b'~')] => (Key::Move(Motion::Home), None),
                [Some(b'['), Some(b'7'), Some(b'~')] => (Key::Move(Motion::Home), None),
                [Some(b'['), Some(b'O'), Some(b'H')] => (Key::Move(Motion::Home), None),
                [Some(b'['), Some(b'H'), pending] => (Key::Move(Motion::Home), *pending),

                [Some(b'['), Some(b'4'), Some(b'~')] => (Key::Move(Motion::End), None),
                [Some(b'['), Some(b'8'), Some(b'~')] => (Key::Move(Motion::End), None),
                [Some(b'['), Some(b'O'), Some(b'F')] => (Key::Move(Motion::End), None),
                [Some(b'['), Some(b'F'), pending] => (Key::Move(Motion::End), *pending),

                [Some(b'['), Some(b'3'), Some(b'~')] => (Key::Delete, None),

                _ => {
                    self.key_buffer.clear();
                    self.key_buffer.extend(seq.iter().rev().filter_map(|&k| k));
                    (self.read_key()?, None)
                }
            };

            if let Some(key) = pending {
                self.key_buffer.push(key);
            }

            key
        } else {
            match key {
                127 => Key::Backspace,
                b'\r' => Key::Newline,
                key if key < 32 => Key::Control((key + 64) as char),
                key => Key::Printable(key),
            }
        })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        Terminal::write("\x1b[2J");
        Terminal::write("\x1b[H");
        self.orig_termios
            .set_attr()
            .expect("Failed to restore terminal state");
    }
}
