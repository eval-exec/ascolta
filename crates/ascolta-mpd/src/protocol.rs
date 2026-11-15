use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

use crate::error::{MpdError, Result};

/// Upper bound for a single response's binary payload, as a guard against a
/// broken server. Album art is capped separately by `binarylimit`.
const MAX_BINARY_BYTES: usize = 32 * 1024 * 1024;

/// A parsed MPD response: ordered `key: value` pairs plus any binary chunks.
#[derive(Debug, Clone, Default)]
pub struct Response {
    pairs: Vec<(String, String)>,
    binary: Vec<u8>,
}

impl Response {
    #[cfg(test)]
    pub(crate) fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        Self {
            pairs,
            binary: Vec::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    pub fn pairs(&self) -> &[(String, String)] {
        &self.pairs
    }

    pub fn binary(&self) -> &[u8] {
        &self.binary
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty() && self.binary.is_empty()
    }
}

/// Quotes one command argument. MPD arguments are double-quoted with `\` and
/// `"` escaped; newlines would terminate the command and are rejected.
pub fn quote(value: &str) -> Result<String> {
    if value.contains(['\n', '\r', '\0']) {
        return Err(MpdError::Protocol(
            "command argument contains a control character".into(),
        ));
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' | '"' => {
                quoted.push('\\');
                quoted.push(character);
            }
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

/// Builds one complete command line, including the trailing newline.
pub fn format_command(command: &str, args: &[&str]) -> Result<String> {
    if command.is_empty() || command.contains(['\n', '\r', ' ']) {
        return Err(MpdError::Protocol(format!(
            "invalid command name: {command:?}"
        )));
    }
    let mut line = String::from(command);
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg)?);
    }
    line.push('\n');
    Ok(line)
}

/// Reads and validates MPD's `OK MPD <version>` greeting.
pub async fn read_greeting<R>(reader: &mut BufReader<R>) -> Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        return Err(MpdError::Closed);
    }
    let line = line.trim_end_matches(['\r', '\n']);
    line.strip_prefix("OK MPD ")
        .map(str::to_owned)
        .ok_or_else(|| MpdError::Protocol(format!("unexpected greeting: {line:?}")))
}

/// Reads one response frame, including binary chunks.
pub async fn read_response<R>(reader: &mut BufReader<R>) -> Result<Response>
where
    R: AsyncRead + Unpin,
{
    let mut pairs = Vec::new();
    let mut binary = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Err(MpdError::Closed);
        }
        let raw = line.trim_end_matches(['\r', '\n']);

        // MPD writes a newline after each binary chunk; it shows up as an
        // empty line here, because the chunk itself was read with read_exact.
        if raw.is_empty() {
            continue;
        }
        if raw == "OK" {
            return Ok(Response { pairs, binary });
        }
        if let Some(rest) = raw.strip_prefix("ACK ") {
            return Err(parse_ack(rest));
        }
        if let Some(size) = raw.strip_prefix("binary: ") {
            let size: usize = size
                .parse()
                .map_err(|_| MpdError::Protocol(format!("invalid binary length: {size:?}")))?;
            if binary.len().saturating_add(size) > MAX_BINARY_BYTES {
                return Err(MpdError::TooLarge(MAX_BINARY_BYTES));
            }
            let start = binary.len();
            binary.resize(start + size, 0);
            reader.read_exact(&mut binary[start..]).await?;
            continue;
        }
        let Some((key, value)) = raw.split_once(':') else {
            return Err(MpdError::Protocol(format!(
                "malformed response line: {raw:?}"
            )));
        };
        pairs.push((
            key.to_owned(),
            value.strip_prefix(' ').unwrap_or(value).to_owned(),
        ));
    }
}

/// Parses the `[code@list_index] {command} message` part of an ACK line.
fn parse_ack(rest: &str) -> MpdError {
    let Some(inner) = rest.strip_prefix('[') else {
        return MpdError::Protocol(format!("malformed ACK: {rest:?}"));
    };
    let Some((location, after)) = inner.split_once(']') else {
        return MpdError::Protocol(format!("malformed ACK: {rest:?}"));
    };
    let code = location
        .split_once('@')
        .map_or(location, |(code, _)| code)
        .parse()
        .unwrap_or(0);

    let after = after.trim_start();
    let (command, message) = match after.strip_prefix('{') {
        Some(rest) => match rest.split_once('}') {
            Some((command, message)) => (command.trim().to_owned(), message.trim().to_owned()),
            None => (String::new(), after.to_owned()),
        },
        None => (String::new(), after.to_owned()),
    };
    MpdError::Ack {
        code,
        command,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, duplex};

    #[test]
    fn quotes_arguments() {
        assert_eq!(quote("hello").unwrap(), "\"hello\"");
        assert_eq!(quote(r#"a "b" \ c"#).unwrap(), r#""a \"b\" \\ c""#);
        assert_eq!(quote("über").unwrap(), "\"über\"");
        assert!(quote("a\nb").is_err());
    }

    #[test]
    fn quoting_roundtrips_arbitrary_arguments() {
        fn unquote(quoted: &str) -> String {
            let inner = &quoted[1..quoted.len() - 1];
            let mut output = String::new();
            let mut characters = inner.chars();
            while let Some(character) = characters.next() {
                if character == '\\' {
                    output.push(characters.next().expect("escaped character"));
                } else {
                    output.push(character);
                }
            }
            output
        }

        let alphabet: Vec<char> = "ab \"\\'./:🎵界_0129".chars().collect();
        let mut state = 0x1234_5678u32;
        for _ in 0..1_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let length = (state as usize) % 32;
            let mut value = String::new();
            for _ in 0..length {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                value.push(alphabet[(state as usize) % alphabet.len()]);
            }
            let quoted = quote(&value).unwrap();
            assert!(quoted.starts_with('"') && quoted.ends_with('"'));
            assert_eq!(unquote(&quoted), value);
            let line = format_command("add", &[&value]).unwrap();
            assert!(line.ends_with('\n'));
        }
        assert!(quote("line\nbreak").is_err());
    }

    #[tokio::test]
    async fn arbitrary_response_frames_never_panic() {
        let alphabet: Vec<char> = "abOKACKbinary: 0129\\\"'/:🎵界-_".chars().collect();
        let mut state = 0x5eed_1234u32;
        for _ in 0..500 {
            let (mut writer, reader) = duplex(64 * 1024);
            let mut frame = String::new();
            let lines = (state as usize % 8) + 1;
            for _ in 0..lines {
                let length = (state as usize) % 40;
                for _ in 0..length {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    frame.push(alphabet[(state as usize) % alphabet.len()]);
                }
                frame.push('\n');
            }
            frame.push_str("OK\n");
            writer.write_all(frame.as_bytes()).await.unwrap();
            writer.shutdown().await.unwrap();

            let mut reader = BufReader::new(reader);
            let _ = read_response(&mut reader).await;
        }
    }

    #[test]
    fn formats_commands() {
        assert_eq!(
            format_command("playid", &["42"]).unwrap(),
            "playid \"42\"\n"
        );
        assert_eq!(format_command("status", &[]).unwrap(), "status\n");
        assert!(format_command("bad command", &[]).is_err());
    }

    #[test]
    fn parses_ack() {
        let error = parse_ack("[50@0] {playid} No such song");
        match error {
            MpdError::Ack {
                code,
                command,
                message,
            } => {
                assert_eq!(code, 50);
                assert_eq!(command, "playid");
                assert_eq!(message, "No such song");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reads_pairs_and_ack() {
        let (client, mut server) = duplex(1024);
        tokio::spawn(async move {
            server
                .write_all(b"volume: 100\nstate: play\nOK\n")
                .await
                .unwrap();
            server
                .write_all(b"ACK [5@0] {find} unknown command\n")
                .await
                .unwrap();
        });

        let mut reader = BufReader::new(client);
        let response = read_response(&mut reader).await.unwrap();
        assert_eq!(response.get("volume"), Some("100"));
        assert_eq!(response.get("state"), Some("play"));

        match read_response(&mut reader).await {
            Err(MpdError::Ack { code, command, .. }) => {
                assert_eq!(code, 5);
                assert_eq!(command, "find");
            }
            other => panic!("unexpected response: {other:?}"),
        }
    }

    #[tokio::test]
    async fn reads_binary_chunks() {
        let (client, mut server) = duplex(1024);
        tokio::spawn(async move {
            server
                .write_all(b"size: 5\ntype: image/png\nbinary: 3\nabc\nbinary: 2\nde\nOK\n")
                .await
                .unwrap();
        });

        let mut reader = BufReader::new(client);
        let response = read_response(&mut reader).await.unwrap();
        assert_eq!(response.get("size"), Some("5"));
        assert_eq!(response.get("type"), Some("image/png"));
        assert_eq!(response.binary(), b"abcde");
    }

    #[tokio::test]
    async fn binary_payload_may_contain_newlines() {
        let (client, mut server) = duplex(1024);
        tokio::spawn(async move {
            server.write_all(b"binary: 3\n\na\n\nOK\n").await.unwrap();
        });

        let mut reader = BufReader::new(client);
        let response = read_response(&mut reader).await.unwrap();
        assert_eq!(response.binary(), b"\na\n");
    }
}
