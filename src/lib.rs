//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

extern crate alloc;

mod common;
#[cfg(feature = "extension")]
mod extension;

pub use crate::common::*;
use libc::{c_char, c_int, c_uchar, c_void};
use linkify::{LinkFinder, LinkKind};
use memchr::{memchr, memchr2_iter, memchr3, memchr_iter, memmem};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[no_mangle]
pub extern "C" fn signal_fts5_tokenize(
    _tokenizer: *mut Fts5Tokenizer,
    p_ctx: *mut c_void,
    _flags: c_int,
    p_text: *const c_char,
    n_text: c_int,
    x_token: TokenFunction,
) -> c_int {
    std::panic::catch_unwind(|| {
        match signal_fts5_tokenize_internal(p_ctx, p_text, n_text, x_token) {
            Ok(()) => SQLITE_OK,
            Err(code) => code,
        }
    })
    .unwrap_or(SQLITE_INTERNAL)
}

#[derive(Debug, PartialEq)]
enum TokenType {
    Normal,
    Synonym,
}

fn on_token(
    p_ctx: *mut c_void,
    x_token: TokenFunction,
    token: &str,
    token_type: TokenType,
    normalized: &mut String,
    start: usize,
    end: usize,
) -> Result<(), c_int> {
    if token.is_empty() {
        return Ok(());
    }

    normalize_into(token, normalized);
    let rc = x_token(
        p_ctx,
        match token_type {
            TokenType::Normal => 0,
            TokenType::Synonym => FTS5_TOKEN_COLOCATED,
        },
        normalized.as_bytes().as_ptr() as *const c_char,
        normalized.len() as c_int,
        start as c_int,
        end as c_int,
    );
    if rc != SQLITE_OK {
        return Err(rc);
    }
    Ok(())
}

fn signal_fts5_tokenize_internal(
    p_ctx: *mut c_void,
    p_text: *const c_char,
    n_text: c_int,
    x_token: TokenFunction,
) -> Result<(), c_int> {
    let slice = unsafe { core::slice::from_raw_parts(p_text as *const c_uchar, n_text as usize) };

    // Map errors to SQLITE_OK because failing here means that the database
    // wouldn't accessible.
    let input = core::str::from_utf8(slice).map_err(|_| SQLITE_OK)?;

    let mut normalized = String::with_capacity(1024);

    let mut finder = LinkFinder::new();
    finder.url_must_have_scheme(false);
    finder.kinds(&[LinkKind::Url]);

    for span in finder.spans(input) {
        match span.kind() {
            Some(LinkKind::Url) => {
                let url = span.as_str();

                // Skip scheme
                let start_off = match memmem::find(url.as_bytes(), b"://") {
                    Some(off) => off + 3,
                    None => 0,
                };

                let start = span.start() + start_off;
                let url = &url[start_off..];

                let (host, path_off, path) = match memchr3(b'/', b'?', b'#', url.as_bytes()) {
                    Some(off) => (&url[..off], off + 1, &url[off + 1..]),
                    None => (url, url.len(), ""),
                };

                let (host, port_off, port) = match memchr(b':', host.as_bytes()) {
                    Some(port_off) => (&host[..port_off], port_off + 1, &host[port_off + 1..]),
                    None => (host, host.len(), ""),
                };

                // Emit full host name
                on_token(
                    p_ctx,
                    x_token,
                    host,
                    TokenType::Normal,
                    &mut normalized,
                    start,
                    start + host.len(),
                )?;

                // Emit host parts: www.youtube.com, youtube.com, com as
                // synonyms
                for off in memchr2_iter(b'.', b'@', host.as_bytes()) {
                    on_token(
                        p_ctx,
                        x_token,
                        &host[(off + 1)..],
                        TokenType::Synonym,
                        &mut normalized,
                        start,
                        start + host.len(),
                    )?;
                }

                // Emit port
                on_token(
                    p_ctx,
                    x_token,
                    port,
                    TokenType::Normal,
                    &mut normalized,
                    start + port_off,
                    start + port_off + port.len(),
                )?;

                let (path, fragment_off, fragment) = match memchr(b'#', path.as_bytes()) {
                    Some(off) => (&path[..off], path_off + off + 1, &path[off + 1..]),
                    None => (path, path.len(), ""),
                };

                let (path, query_off, query) = match memchr(b'?', path.as_bytes()) {
                    Some(off) => (&path[..off], path_off + off + 1, &path[off + 1..]),
                    None => (path, path.len(), ""),
                };

                let mut last_off: usize = 0;
                for off in memchr_iter(b'/', path.as_bytes()).chain(std::iter::once(path.len())) {
                    on_token(
                        p_ctx,
                        x_token,
                        &path[last_off..off],
                        TokenType::Normal,
                        &mut normalized,
                        start + path_off + last_off,
                        start + path_off + off,
                    )?;
                    last_off = off + 1;
                }
                on_token(
                    p_ctx,
                    x_token,
                    query,
                    TokenType::Normal,
                    &mut normalized,
                    start + query_off,
                    start + query_off + query.len(),
                )?;
                on_token(
                    p_ctx,
                    x_token,
                    fragment,
                    TokenType::Normal,
                    &mut normalized,
                    start + fragment_off,
                    start + fragment_off + fragment.len(),
                )?;
            }
            _ => {
                for (off, segment) in span.as_str().unicode_word_indices() {
                    on_token(
                        p_ctx,
                        x_token,
                        segment,
                        TokenType::Normal,
                        &mut normalized,
                        span.start() + off,
                        span.start() + off + segment.len(),
                    )?;
                }
            }
        }
    }

    Ok(())
}

fn is_diacritic(x: char) -> bool {
    ('\u{0300}'..='\u{036f}').contains(&x)
}

fn normalize_into(segment: &str, buf: &mut String) {
    buf.clear();

    for x in segment.nfd() {
        if is_diacritic(x) {
            continue;
        }
        if x.is_ascii() {
            buf.push(x.to_ascii_lowercase());
        } else {
            buf.extend(x.to_lowercase());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_normalizes_segment() {
        let mut buf = String::new();
        normalize_into("DïācRîtįcs", &mut buf);
        assert_eq!(buf, "diacritics");
    }

    extern "C" fn token_callback(
        ctx: *mut c_void,
        flags: c_int,
        token: *const c_char,
        token_len: c_int,
        start: c_int,
        end: c_int,
    ) -> c_int {
        let tokens_ptr = ctx as *mut _ as *mut Vec<(TokenType, String, c_int, c_int)>;
        let tokens = unsafe { tokens_ptr.as_mut() }.expect("tokens pointer");
        let slice =
            unsafe { core::slice::from_raw_parts(token as *const c_uchar, token_len as usize) };
        let token = String::from_utf8(slice.to_vec()).expect("Expected utf-8 token");

        let token_type = match flags {
            0 => TokenType::Normal,
            FTS5_TOKEN_COLOCATED => TokenType::Synonym,
            _ => panic!("Invalid token flag {}", flags),
        };

        tokens.push((token_type, token, start, end));

        return SQLITE_OK;
    }

    #[test]
    fn it_emits_segments() {
        let input = "hello world! 知识? 안녕 세상";
        let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];
        signal_fts5_tokenize_internal(
            &mut tokens as *mut _ as *mut c_void,
            input.as_bytes().as_ptr() as *const c_char,
            input.len() as i32,
            token_callback,
        )
        .expect("tokenize internal should not fail");

        assert_eq!(
            tokens,
            [
                (TokenType::Normal, "hello", 0, 5),
                (TokenType::Normal, "world", 6, 11),
                (TokenType::Normal, "知", 13, 16),
                (TokenType::Normal, "识", 16, 19),
                (TokenType::Normal, "안녕", 21, 27),
                (TokenType::Normal, "세상", 28, 34)
            ]
            .map(|(token_type, s, start, end)| (token_type, s.to_owned(), start, end))
        );
    }

    #[test]
    fn it_ignores_invalid_utf8() {
        let input = b"\xc3\x28";
        let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];

        assert_eq!(
            signal_fts5_tokenize_internal(
                &mut tokens as *mut _ as *mut c_void,
                input.as_ptr() as *const c_char,
                input.len() as i32,
                token_callback,
            )
            .expect_err("tokenize internal should not fail"),
            SQLITE_OK
        );

        assert_eq!(tokens, []);
    }

    #[test]
    fn it_tokenizes_urls() {
        let test_vectors = vec![
            (
                "www.example.com",
                vec!["www.example.com", "example.com", "com"],
            ),
            ("example.com?abc", vec!["example.com", "com", "abc"]),
            ("example.com#abc", vec!["example.com", "com", "abc"]),
            (
                "example.com?abc#def",
                vec!["example.com", "com", "abc", "def"],
            ),
            (
                "example.com:123/abc?def",
                vec!["example.com", "com", "123", "abc", "def"],
            ),
        ];

        for (input, expected_tokens) in test_vectors {
            let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];
            signal_fts5_tokenize_internal(
                &mut tokens as *mut _ as *mut c_void,
                input.as_bytes().as_ptr() as *const c_char,
                input.len() as i32,
                token_callback,
            )
            .expect("tokenize internal should not fail");

            assert_eq!(
                tokens
                    .into_iter()
                    .map(|(_token_type, s, _start, _end)| s)
                    .collect::<Vec<_>>(),
                expected_tokens,
            );
        }
    }

    #[test]
    fn it_tokenizes_urls_with_correct_offsets() {
        let input = "See https://www.signal.org:443/abc/def?q=1##hello/world for details";
        let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];
        signal_fts5_tokenize_internal(
            &mut tokens as *mut _ as *mut c_void,
            input.as_bytes().as_ptr() as *const c_char,
            input.len() as i32,
            token_callback,
        )
        .expect("tokenize internal should not fail");

        assert_eq!(
            tokens,
            [
                (TokenType::Normal, "see", 0, 3),
                (TokenType::Normal, "www.signal.org", 12, 26),
                (TokenType::Synonym, "signal.org", 12, 26),
                (TokenType::Synonym, "org", 12, 26),
                (TokenType::Normal, "443", 27, 30),
                (TokenType::Normal, "abc", 31, 34),
                (TokenType::Normal, "def", 35, 38),
                (TokenType::Normal, "q=1", 39, 42),
                (TokenType::Normal, "#hello/world", 43, 55),
                (TokenType::Normal, "for", 56, 59),
                (TokenType::Normal, "details", 60, 67),
            ]
            .map(|(token_type, s, start, end)| (token_type, s.to_owned(), start, end))
        );
    }
}
