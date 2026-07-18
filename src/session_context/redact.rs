//! Secret redaction applied to every piece of session-context text.
//!
//! Redaction MUST run before excerpting/truncation so a cut can never split a
//! secret in a way that defeats pattern recognition. There is deliberately no
//! unredacted mode.

/// Redacts obvious secrets line by line. Multi-line private-key blocks are
/// collapsed as a whole.
pub fn redact(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_private_key = false;
    for line in s.lines() {
        if in_private_key {
            if line.contains("-----END") {
                in_private_key = false;
                out.push(line.to_string());
            } else {
                out.push("[REDACTED]".to_string());
            }
            continue;
        }
        if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            in_private_key = true;
            out.push(line.to_string());
            continue;
        }
        out.push(redact_line(line));
    }
    out.join("\n")
}

fn redact_line(line: &str) -> String {
    let line = redact_connection_string_parts(line);
    let line = redact_url_credentials(&line);
    let trimmed = line.trim_start();
    let lower = trimmed.to_lowercase();
    if contains_secret_key(&lower) && (trimmed.contains('=') || trimmed.contains(':')) {
        if let Some(pos) = line.find('=') {
            return format!("{}= [REDACTED]", &line[..pos].trim_end());
        }
        if let Some(pos) = line.find(':') {
            return format!("{}: [REDACTED]", &line[..pos].trim_end());
        }
    }

    line.split_whitespace()
        .map(redact_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_secret_key(lower: &str) -> bool {
    [
        "api_key",
        "apikey",
        "auth_token",
        "access_token",
        "refresh_token",
        "password",
        "passwd",
        "secret",
        "credential",
        "bearer ",
        "authorization",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

fn redact_token(token: &str) -> String {
    let stripped = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
    let sensitive = stripped.starts_with("sk-")
        || stripped.starts_with("ghp_")
        || stripped.starts_with("github_pat_")
        || stripped.starts_with("xoxb-")
        || (stripped.starts_with("AKIA") && stripped.len() >= 16)
        || is_jwt_shape(stripped);
    if sensitive {
        token.replace(stripped, "[REDACTED]")
    } else {
        token.to_string()
    }
}

/// Common JWT shape: three dot-separated base64url segments, header starting with
/// `eyJ` (base64 of `{"`).
fn is_jwt_shape(token: &str) -> bool {
    token.starts_with("eyJ")
        && token.matches('.').count() == 2
        && token.len() >= 20
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '='))
}

/// Masks the password part of `scheme://user:password@host` URL credentials.
fn redact_url_credentials(line: &str) -> String {
    let mut out = line.to_string();
    let mut search = 0usize;
    while let Some(rel) = out[search..].find("://") {
        let start = search + rel + 3;
        let seg_end = out[start..]
            .find(|c: char| c == '/' || c.is_whitespace())
            .map(|r| start + r)
            .unwrap_or(out.len());
        // Only the userinfo before the last '@' of the authority can carry a password.
        if let Some(at) = out[start..seg_end].rfind('@') {
            if let Some(colon) = out[start..start + at].find(':') {
                let pw_start = start + colon + 1;
                let pw_end = start + at;
                if pw_start < pw_end {
                    out.replace_range(pw_start..pw_end, "[REDACTED]");
                    search = pw_start + "[REDACTED]".len() + 1;
                    continue;
                }
            }
        }
        search = seg_end.max(start);
        if search >= out.len() {
            break;
        }
    }
    out
}

/// Masks Azure-style `SharedAccessKey=...` values inside connection strings.
fn redact_connection_string_parts(line: &str) -> String {
    let mut out = line.to_string();
    for key in ["SharedAccessKey=", "sharedAccessKey="] {
        let mut search_from = 0usize;
        while let Some(rel) = out[search_from..].find(key) {
            let pos = search_from + rel;
            let value_start = pos + key.len();
            let value_end = out[value_start..]
                .find([';', '"', '\'', ' ', '\t', '\n', '\\'])
                .map(|rel| value_start + rel)
                .unwrap_or_else(|| out.len());
            out.replace_range(value_start..value_end, "[REDACTED]");
            search_from = (value_start + "[REDACTED]".len()).min(out.len());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_key_value_secrets() {
        assert_eq!(redact("API_KEY=sk-abc1234567890"), "API_KEY= [REDACTED]");
        assert_eq!(redact("password: hunter2"), "password: [REDACTED]");
    }

    #[test]
    fn redacts_prefixed_tokens() {
        let out = redact("token is ghp_0123456789abcdef here");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("ghp_"));
    }

    #[test]
    fn redacts_authorization_header() {
        let out = redact("Authorization: Bearer abc.def.ghi");
        assert_eq!(out, "Authorization: [REDACTED]");
    }

    #[test]
    fn redacts_private_key_block_body() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\nQ29udGVudA==\n-----END RSA PRIVATE KEY-----";
        let out = redact(text);
        assert!(out.contains("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(out.contains("-----END RSA PRIVATE KEY-----"));
        assert!(!out.contains("MIIEowIBAAKCAQEA"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_url_password_only() {
        let out = redact("postgres://user:s3cr3t@db.example.com:5432/app");
        assert_eq!(out, "postgres://user:[REDACTED]@db.example.com:5432/app");
        // URLs without credentials stay untouched.
        assert_eq!(
            redact("https://example.com/a:b@c"),
            "https://example.com/a:b@c"
        );
    }

    #[test]
    fn redacts_jwt_shaped_tokens() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.TJVA95OrM7E2cBab30RMHrHDcEfxjoYZgeFONFh7HgQ";
        let out = redact(&format!("token {jwt} end"));
        assert!(!out.contains("eyJhbGci"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_shared_access_key() {
        let out = redact("Endpoint=sb://x.net/;SharedAccessKey=abc123;EntityPath=q");
        assert!(out.contains("SharedAccessKey=[REDACTED];"));
        assert!(!out.contains("abc123"));
    }
}
