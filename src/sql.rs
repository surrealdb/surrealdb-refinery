//! Minimal SurrealQL text handling.

/// Strip an outer `BEGIN` / `COMMIT` pair from a migration body.
///
/// The driver runs every migration inside a transaction it manages itself, and
/// SurrealDB rejects a nested `BEGIN` with "Cannot BEGIN a transaction within a
/// transaction". Migrations written in the older style, which wrap their own
/// body, would therefore fail. Unwrapping them keeps those migrations working
/// with identical semantics.
///
/// This is deliberately conservative: the pair is removed only when `BEGIN` is
/// the very first token and a matching `COMMIT` is the very last one. A `COMMIT`
/// appearing inside a string literal is not treated as the closing keyword,
/// because the character preceding it must be whitespace or a semicolon.
pub(crate) fn strip_outer_transaction(sql: &str) -> &str {
    let start = skip_trivia(sql, 0);
    let Some(after_begin) = keyword_at(sql, start, "BEGIN") else {
        return sql;
    };

    // `BEGIN` may be spelled `BEGIN TRANSACTION`.
    let mut cursor = after_begin;
    let next = skip_trivia(sql, cursor);
    if let Some(end) = keyword_at(sql, next, "TRANSACTION") {
        cursor = end;
    }

    // The statement separator is optional in the grammar.
    let next = skip_trivia(sql, cursor);
    let body_start = if sql.as_bytes().get(next) == Some(&b';') {
        next + 1
    } else {
        cursor
    };

    match trailing_commit(sql, body_start) {
        Some(body_end) => &sql[body_start..body_end],
        // An opening `BEGIN` with no closing `COMMIT` is malformed; leave it
        // alone so SurrealDB reports it rather than this function hiding it.
        None => sql,
    }
}

/// Advance past whitespace, `--` and `#` line comments, and `/* */` blocks.
fn skip_trivia(sql: &str, mut index: usize) -> usize {
    let bytes = sql.as_bytes();
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let rest = &sql[index.min(sql.len())..];
        if rest.starts_with("--") || rest.starts_with('#') {
            match rest.find('\n') {
                Some(offset) => index += offset + 1,
                None => return bytes.len(),
            }
            continue;
        }
        if let Some(after_open) = rest.strip_prefix("/*") {
            match after_open.find("*/") {
                Some(offset) => index += 2 + offset + 2,
                None => return bytes.len(),
            }
            continue;
        }
        return index;
    }
}

/// Match `keyword` at `index`, case-insensitively, on a word boundary.
///
/// Returns the index just past the keyword. Compares bytes so a multi-byte
/// character at `index` can never cause a panic.
fn keyword_at(sql: &str, index: usize, keyword: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let end = index + keyword.len();
    if end > bytes.len() || !bytes[index..end].eq_ignore_ascii_case(keyword.as_bytes()) {
        return None;
    }
    if let Some(&next) = bytes.get(end)
        && (next.is_ascii_alphanumeric() || next == b'_')
    {
        return None;
    }
    Some(end)
}

/// Find the index at which a trailing `COMMIT [TRANSACTION] [;]` begins.
fn trailing_commit(sql: &str, from: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let trim = |mut end: usize| {
        while end > from && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        end
    };

    let mut end = trim(bytes.len());
    if end > from && bytes[end - 1] == b';' {
        end = trim(end - 1);
    }

    // `COMMIT` may be spelled `COMMIT TRANSACTION`.
    if let Some(start) = ends_with_keyword(bytes, from, end, b"TRANSACTION") {
        end = trim(start);
    }

    ends_with_keyword(bytes, from, end, b"COMMIT")
}

/// If `bytes[from..end]` ends with `keyword` on a word boundary, return the
/// index where the keyword starts.
fn ends_with_keyword(bytes: &[u8], from: usize, end: usize, keyword: &[u8]) -> Option<usize> {
    let start = end.checked_sub(keyword.len())?;
    if start < from || !bytes[start..end].eq_ignore_ascii_case(keyword) {
        return None;
    }
    // Reject a keyword that is part of a longer token or a string literal.
    if start > from {
        let previous = bytes[start - 1];
        if !previous.is_ascii_whitespace() && previous != b';' {
            return None;
        }
    }
    Some(start)
}

#[cfg(test)]
mod tests {
    use super::strip_outer_transaction;

    #[test]
    fn leaves_a_plain_body_alone() {
        let sql = "DEFINE TABLE users SCHEMAFULL;";
        assert_eq!(strip_outer_transaction(sql), sql);
    }

    #[test]
    fn strips_a_wrapping_transaction() {
        let sql = "BEGIN;\nDEFINE TABLE users SCHEMAFULL;\nCOMMIT;";
        assert_eq!(
            strip_outer_transaction(sql).trim(),
            "DEFINE TABLE users SCHEMAFULL;"
        );
    }

    #[test]
    fn strips_past_a_leading_comment() {
        let sql = "-- set up users\nBEGIN;\nDEFINE TABLE users;\nCOMMIT;";
        assert_eq!(strip_outer_transaction(sql).trim(), "DEFINE TABLE users;");
    }

    #[test]
    fn strips_past_a_leading_block_comment() {
        let sql = "/* multi\nline */ BEGIN; DEFINE TABLE users; COMMIT;";
        assert_eq!(strip_outer_transaction(sql).trim(), "DEFINE TABLE users;");
    }

    #[test]
    fn handles_the_transaction_keyword_and_case() {
        let sql = "begin transaction; DEFINE TABLE users; commit transaction;";
        assert_eq!(strip_outer_transaction(sql).trim(), "DEFINE TABLE users;");
    }

    #[test]
    fn tolerates_a_missing_trailing_semicolon() {
        let sql = "BEGIN; DEFINE TABLE users; COMMIT";
        assert_eq!(strip_outer_transaction(sql).trim(), "DEFINE TABLE users;");
    }

    #[test]
    fn does_not_treat_a_string_literal_as_the_closing_keyword() {
        // The trailing COMMIT here is inside quotes, so the body is malformed
        // rather than wrapped, and must be handed to SurrealDB untouched.
        let sql = "BEGIN; CREATE note SET body = 'COMMIT';";
        assert_eq!(strip_outer_transaction(sql), sql);
    }

    #[test]
    fn keeps_an_inner_commit_that_is_not_last() {
        let sql = "BEGIN; CREATE note SET body = 'COMMIT'; DELETE note; COMMIT;";
        assert_eq!(
            strip_outer_transaction(sql).trim(),
            "CREATE note SET body = 'COMMIT'; DELETE note;"
        );
    }

    #[test]
    fn leaves_an_unclosed_transaction_alone() {
        let sql = "BEGIN; DEFINE TABLE users;";
        assert_eq!(strip_outer_transaction(sql), sql);
    }

    #[test]
    fn ignores_begin_that_is_not_the_first_token() {
        let sql = "DEFINE TABLE users; BEGIN; DEFINE TABLE roles; COMMIT;";
        assert_eq!(strip_outer_transaction(sql), sql);
    }

    #[test]
    fn ignores_an_identifier_that_merely_starts_with_begin() {
        let sql = "CREATE beginning SET at = 1;";
        assert_eq!(strip_outer_transaction(sql), sql);
    }

    #[test]
    fn handles_an_empty_body() {
        assert_eq!(strip_outer_transaction("BEGIN; COMMIT;").trim(), "");
        assert_eq!(strip_outer_transaction(""), "");
    }

    #[test]
    fn handles_multibyte_characters() {
        let sql = "BEGIN; CREATE note SET body = 'héllo → wörld'; COMMIT;";
        assert_eq!(
            strip_outer_transaction(sql).trim(),
            "CREATE note SET body = 'héllo → wörld';"
        );
    }
}
