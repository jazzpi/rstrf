use anyhow::Context;
use tokio::io::AsyncBufReadExt;

enum ParseState {
    AwaitLine1OrTitle,
    AwaitLine1(String),
    AwaitLine2(Option<String>, String),
}

/// Loads TLEs from the given file
///
/// Parses 2LE, and 3LE with an optional initial 0 in the title line.
pub async fn load(path: &std::path::PathBuf) -> anyhow::Result<Vec<Satellite>> {
    let file = tokio::fs::File::open(path).await?;
    let reader = tokio::io::BufReader::new(file);
    let mut state = ParseState::AwaitLine1OrTitle;
    let mut elements = Vec::new();
    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await? {
        state = match state {
            ParseState::AwaitLine1OrTitle => {
                if line.starts_with("1 ") {
                    ParseState::AwaitLine2(None, line)
                } else if line.starts_with("0 ") {
                    let title = line[2..].to_string();
                    ParseState::AwaitLine1(title)
                } else {
                    ParseState::AwaitLine1(line)
                }
            }
            ParseState::AwaitLine1(title) => {
                anyhow::ensure!(
                    line.starts_with("1 "),
                    "Expected line 1 of TLE, got: {}",
                    line
                );
                ParseState::AwaitLine2(Some(title), line)
            }
            ParseState::AwaitLine2(title, line1) => {
                anyhow::ensure!(
                    line.starts_with("2 "),
                    "Expected line 2 of TLE, got: {}",
                    line
                );
                let elem = sgp4::Elements::from_tle(title, line1.as_bytes(), line.as_bytes())
                    .context("Failed to parse TLE")?;
                let constants = sgp4::Constants::from_elements(&elem)
                    .context("Failed to derive SGP4 constants")?;
                elements.push(Satellite {
                    elements: elem,
                    constants,
                });
                ParseState::AwaitLine1OrTitle
            }
        };
    }
    Ok(elements)
}

#[derive(Debug, Clone)]
pub struct Satellite {
    elements: sgp4::Elements,
    constants: sgp4::Constants,
}
