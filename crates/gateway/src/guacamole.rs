//! RDP and VNC through guacd (ADR 0003): the Guacamole protocol's handshake
//! and the framing of instructions for the tunnel to the browser.
//!
//! An instruction is a list of elements `LENGTH.VALUE`, separated by `,` and
//! ended by `;`; the first element is the opcode. `LENGTH` counts Unicode
//! code points, not bytes, so a read can end in the middle of a character.
//!
//! The server opens the connection: `select` names the protocol, guacd answers
//! with the parameter names it takes (`args`), the server sends the display
//! and the values — credentials included — in `connect`, and guacd confirms
//! with `ready`. From then on the tunnel relays instructions; the browser
//! never learns the parameters.

use std::time::Duration;

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

/// The protocol version remotehub speaks (guacd 1.6 announces `VERSION_1_5_0`).
pub const PROTOCOL_VERSION: &str = "VERSION_1_5_0";

/// guacd's limits (`GUAC_INSTRUCTION_MAX_LENGTH`, `_MAX_ELEMENTS`); anything
/// larger is not Guacamole.
const MAX_INSTRUCTION_BYTES: usize = 32 * 1024;
const MAX_ELEMENTS: usize = 128;

#[derive(Debug, Error)]
pub enum GuacError {
    #[error("guacd is not reachable: {0}")]
    Unreachable(std::io::Error),
    #[error("guacd did not answer in time")]
    Timeout,
    #[error("guacd refused the connection: {message} ({status})")]
    Refused { message: String, status: u16 },
    #[error("not a Guacamole instruction: {0}")]
    Malformed(&'static str),
    #[error("connection to guacd lost: {0}")]
    Io(#[from] std::io::Error),
}

/// One instruction: opcode and arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub opcode: String,
    pub args: Vec<String>,
}

impl Instruction {
    pub fn new<I, S>(opcode: &str, args: I) -> Instruction
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Instruction {
            opcode: opcode.to_owned(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    pub fn encode(&self) -> String {
        let mut out = String::new();
        encode_into(&mut out, &self.opcode, self.args.iter().map(String::as_str));
        out
    }
}

fn encode_into<'a>(out: &mut String, opcode: &str, args: impl Iterator<Item = &'a str>) {
    push_element(out, opcode);
    for arg in args {
        out.push(',');
        push_element(out, arg);
    }
    out.push(';');
}

fn push_element(out: &mut String, value: &str) {
    out.push_str(&value.chars().count().to_string());
    out.push('.');
    out.push_str(value);
}

/// Splits a byte stream into complete instructions.
#[derive(Default)]
pub struct Parser {
    buffer: Vec<u8>,
}

impl Parser {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next complete instruction, if the buffer holds one.
    pub fn next_instruction(&mut self) -> Result<Option<Instruction>, GuacError> {
        let Some((end, elements)) = scan(&self.buffer)? else {
            return Ok(None);
        };
        let mut values = elements.into_iter().map(|(start, stop)| {
            // `scan` only returns boundaries of complete UTF-8 characters.
            String::from_utf8(self.buffer[start..stop].to_vec())
                .map_err(|_| GuacError::Malformed("invalid UTF-8"))
        });
        let opcode = values
            .next()
            .ok_or(GuacError::Malformed("empty instruction"))??;
        let args = values.collect::<Result<Vec<_>, _>>()?;
        self.buffer.drain(..end);
        Ok(Some(Instruction { opcode, args }))
    }

    /// All complete instructions in the buffer, unchanged, as one text — what
    /// the browser gets in one WebSocket frame. The browser's client needs
    /// whole instructions per frame.
    pub fn take_complete(&mut self) -> Result<Option<String>, GuacError> {
        let mut end = 0;
        while let Some((length, _)) = scan(&self.buffer[end..])? {
            end += length;
        }
        if end == 0 {
            return Ok(None);
        }
        let text = String::from_utf8(self.buffer.drain(..end).collect())
            .map_err(|_| GuacError::Malformed("invalid UTF-8"))?;
        Ok(Some(text))
    }
}

/// A complete instruction found in a buffer: its length in bytes and the
/// byte ranges of its elements.
type Scanned = (usize, Vec<(usize, usize)>);

/// Finds the first complete instruction; `None` if it is not complete yet.
fn scan(buffer: &[u8]) -> Result<Option<Scanned>, GuacError> {
    let mut elements = Vec::new();
    let mut at = 0;
    loop {
        // LENGTH
        let digits = buffer[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 5 {
            return Err(GuacError::Malformed("element too long"));
        }
        if at + digits >= buffer.len() {
            return incomplete(buffer);
        }
        if digits == 0 || buffer[at + digits] != b'.' {
            return Err(GuacError::Malformed("element without length"));
        }
        let length: usize = std::str::from_utf8(&buffer[at..at + digits])
            .ok()
            .and_then(|d| d.parse().ok())
            .ok_or(GuacError::Malformed("element without length"))?;
        at += digits + 1;

        // VALUE: `length` characters.
        let start = at;
        for _ in 0..length {
            let Some(&lead) = buffer.get(at) else {
                return incomplete(buffer);
            };
            at += match lead {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                _ => return Err(GuacError::Malformed("invalid UTF-8")),
            };
        }
        let Some(&separator) = buffer.get(at) else {
            return incomplete(buffer);
        };
        elements.push((start, at));
        if elements.len() > MAX_ELEMENTS {
            return Err(GuacError::Malformed("too many elements"));
        }
        at += 1;
        match separator {
            b';' => return Ok(Some((at, elements))),
            b',' => {}
            _ => return Err(GuacError::Malformed("element without separator")),
        }
    }
}

fn incomplete<T>(buffer: &[u8]) -> Result<Option<T>, GuacError> {
    if buffer.len() > MAX_INSTRUCTION_BYTES {
        return Err(GuacError::Malformed("instruction too long"));
    }
    Ok(None)
}

/// Instructions the browser may send once the connection is up: input,
/// display size, clipboard and the acknowledgements of streams. Everything
/// else — `argv` (changing connection parameters), file transfer, pipes — is
/// dropped, whatever the browser claims.
pub fn allowed_from_browser(opcode: &str) -> bool {
    matches!(
        opcode,
        "ack"
            | "blob"
            | "clipboard"
            | "disconnect"
            | "end"
            | "key"
            | "mouse"
            | "nop"
            | "size"
            | "sync"
            | "touch"
    )
}

/// What to open: the protocol, its parameters (by guacd's names) and the
/// browser's display.
pub struct Handshake<'a> {
    /// `rdp` or `vnc`.
    pub protocol: &'a str,
    /// Values by parameter name; parameters guacd asks for but that are not
    /// listed stay empty (guacd's default). Holds the credentials.
    pub parameters: &'a [(&'a str, &'a str)],
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub timezone: Option<&'a str>,
}

/// A connection through guacd after `ready`.
pub struct Connection {
    stream: TcpStream,
    parser: Parser,
    /// guacd's connection ID, e.g. `$0b8f…`.
    pub id: String,
}

/// Opens a connection through guacd at `address` (`host:port`).
pub async fn open(
    address: &str,
    handshake: &Handshake<'_>,
    timeout: Duration,
) -> Result<Connection, GuacError> {
    tokio::time::timeout(timeout, open_now(address, handshake))
        .await
        .map_err(|_| GuacError::Timeout)?
}

async fn open_now(address: &str, handshake: &Handshake<'_>) -> Result<Connection, GuacError> {
    let stream = TcpStream::connect(address)
        .await
        .map_err(GuacError::Unreachable)?;
    stream.set_nodelay(true)?;
    let mut connection = Connection {
        stream,
        parser: Parser::default(),
        id: String::new(),
    };

    connection
        .send(&Instruction::new("select", [handshake.protocol]).encode())
        .await?;
    let args = connection.expect("args").await?;

    let mut setup = String::new();
    let (width, height, dpi) = (
        handshake.width.to_string(),
        handshake.height.to_string(),
        handshake.dpi.to_string(),
    );
    encode_into(
        &mut setup,
        "size",
        [width.as_str(), height.as_str(), dpi.as_str()].into_iter(),
    );
    encode_into(&mut setup, "audio", std::iter::empty::<&str>());
    encode_into(&mut setup, "video", std::iter::empty::<&str>());
    encode_into(
        &mut setup,
        "image",
        ["image/png", "image/jpeg", "image/webp"].into_iter(),
    );
    if let Some(timezone) = handshake.timezone {
        encode_into(&mut setup, "timezone", std::iter::once(timezone));
    }
    connection.send(&setup).await?;

    // The values in the order guacd asked for them. The instruction holds the
    // credentials: wiped once it is sent.
    let values = args.args.iter().map(|name| {
        if name.starts_with("VERSION_") {
            PROTOCOL_VERSION
        } else {
            handshake
                .parameters
                .iter()
                .find(|(key, _)| key == name)
                .map_or("", |(_, value)| *value)
        }
    });
    let mut connect = Zeroizing::new(String::new());
    encode_into(&mut connect, "connect", values);
    connection.send(&connect).await?;
    drop(connect);

    let ready = connection.expect("ready").await?;
    connection.id = ready.args.into_iter().next().unwrap_or_default();
    Ok(connection)
}

impl Connection {
    /// Waits for an instruction with this opcode; `error` from guacd ends the
    /// handshake.
    async fn expect(&mut self, opcode: &str) -> Result<Instruction, GuacError> {
        loop {
            while let Some(instruction) = self.parser.next_instruction()? {
                if instruction.opcode == opcode {
                    return Ok(instruction);
                }
                if instruction.opcode == "error" {
                    return Err(refused(&instruction));
                }
            }
            self.fill().await?;
        }
    }

    async fn fill(&mut self) -> Result<(), GuacError> {
        let mut chunk = [0u8; 16 * 1024];
        let read = self.stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(GuacError::Io(std::io::ErrorKind::UnexpectedEof.into()));
        }
        self.parser.push(&chunk[..read]);
        Ok(())
    }

    /// Complete instructions from guacd as one text, `None` once guacd has
    /// closed the connection. Cancel-safe: nothing read is lost if the
    /// future is dropped.
    pub async fn receive(&mut self) -> Result<Option<String>, GuacError> {
        loop {
            if let Some(text) = self.parser.take_complete()? {
                return Ok(Some(text));
            }
            let mut chunk = [0u8; 16 * 1024];
            let read = self.stream.read(&mut chunk).await?;
            if read == 0 {
                return Ok(None);
            }
            self.parser.push(&chunk[..read]);
        }
    }

    /// Sends encoded instructions to guacd.
    pub async fn send(&mut self, instructions: &str) -> Result<(), GuacError> {
        self.stream.write_all(instructions.as_bytes()).await?;
        Ok(())
    }

    /// Ends the connection politely.
    pub async fn close(mut self) {
        let _ = self
            .send(&Instruction::new("disconnect", Vec::<String>::new()).encode())
            .await;
        let _ = self.stream.shutdown().await;
    }
}

fn refused(error: &Instruction) -> GuacError {
    GuacError::Refused {
        message: error.args.first().cloned().unwrap_or_default(),
        status: error.args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_count_characters_not_bytes() {
        let instruction = Instruction::new("clipboard", ["Grüße", "😀"]);
        assert_eq!(instruction.encode(), "9.clipboard,5.Grüße,1.😀;");
        let mut parser = Parser::default();
        parser.push(instruction.encode().as_bytes());
        assert_eq!(parser.next_instruction().unwrap(), Some(instruction));
        assert_eq!(parser.next_instruction().unwrap(), None);
    }

    #[test]
    fn separators_inside_values_are_data() {
        let mut parser = Parser::default();
        parser.push(b"4.name,5.a;b,c;");
        let instruction = parser.next_instruction().unwrap().unwrap();
        assert_eq!(instruction.args, ["a;b,c"]);
    }

    #[test]
    fn instructions_arrive_in_pieces() {
        let text = "4.sync,8.12345678;5.mouse,1.1,1.2;";
        let bytes = "3.key,1.ä,1.1;".as_bytes();
        let mut parser = Parser::default();
        // Every split point, including inside the two bytes of `ä`.
        for split in 0..bytes.len() {
            parser.push(&bytes[..split]);
            assert_eq!(parser.take_complete().unwrap(), None, "split at {split}");
            parser.push(&bytes[split..]);
            assert_eq!(
                parser.take_complete().unwrap().as_deref(),
                Some("3.key,1.ä,1.1;")
            );
        }
        parser.push(text.as_bytes());
        parser.push(b"3.nop");
        assert_eq!(parser.take_complete().unwrap().as_deref(), Some(text));
        parser.push(b";");
        assert_eq!(parser.take_complete().unwrap().as_deref(), Some("3.nop;"));
    }

    #[test]
    fn garbage_is_refused() {
        for garbage in [
            &b"hello;"[..],
            b"3.abc.",
            b"x.abc;",
            b"3.ab;x",
            b"123456.a;",
        ] {
            let mut parser = Parser::default();
            parser.push(garbage);
            assert!(
                parser.next_instruction().is_err(),
                "{:?}",
                String::from_utf8_lossy(garbage)
            );
        }
        let mut parser = Parser::default();
        parser.push(format!("{}.", MAX_INSTRUCTION_BYTES).as_bytes());
        parser.push(&vec![b'a'; MAX_INSTRUCTION_BYTES]);
        assert!(parser.take_complete().is_err());
    }

    #[test]
    fn the_browser_may_only_send_input_and_stream_control() {
        for opcode in [
            "key",
            "mouse",
            "size",
            "sync",
            "clipboard",
            "blob",
            "end",
            "ack",
        ] {
            assert!(allowed_from_browser(opcode), "{opcode}");
        }
        for opcode in [
            "argv", "file", "pipe", "put", "get", "connect", "select", "",
        ] {
            assert!(!allowed_from_browser(opcode), "{opcode}");
        }
    }
}
