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
use memchr::{memchr, memchr3, memchr_iter, memmem};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

struct TokenContext {
    p_ctx: *mut c_void,
    x_token: TokenFunction,
    normalized: String,
}

fn get_slice_range(src: &str, slice: &str) -> core::range::Range<usize> {
    let src_start = src.as_ptr().addr();
    let slice_start = slice.as_ptr().addr();

    let start = slice_start.wrapping_sub(src_start);
    let end = start.wrapping_add(slice.len());

    assert!(start <= src.len() && end <= src.len());
    core::range::Range { start, end }
}

#[no_mangle]
pub extern "C" fn signal_fts5_tokenize(
    _tokenizer: *mut Fts5Tokenizer,
    p_ctx: *mut c_void,
    flags: c_int,
    p_text: *const c_char,
    n_text: c_int,
    x_token: TokenFunction,
) -> c_int {
    std::panic::catch_unwind(|| {
        match signal_fts5_tokenize_internal(p_ctx, flags, p_text, n_text, x_token) {
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

fn on_raw_token(
    ctx: &mut TokenContext,
    token: &str,
    token_type: TokenType,
    range: &core::range::Range<usize>,
) -> Result<(), c_int> {
    if token.is_empty() {
        return Ok(());
    }

    normalize_into(token, &mut ctx.normalized);
    let rc = (ctx.x_token)(
        ctx.p_ctx,
        match token_type {
            TokenType::Normal => 0,
            TokenType::Synonym => FTS5_TOKEN_COLOCATED,
        },
        ctx.normalized.as_bytes().as_ptr() as *const c_char,
        ctx.normalized.len() as c_int,
        range.start as c_int,
        range.end as c_int,
    );
    if rc != SQLITE_OK {
        return Err(rc);
    }
    Ok(())
}

fn on_token(ctx: &mut TokenContext, src: &str, token: &str) -> Result<(), c_int> {
    for (_off, segment) in token.unicode_word_indices() {
        let range = get_slice_range(src, segment);
        on_raw_token(ctx, segment, TokenType::Normal, &range)?;
    }
    Ok(())
}

fn signal_fts5_tokenize_internal(
    p_ctx: *mut c_void,
    flags: c_int,
    p_text: *const c_char,
    n_text: c_int,
    x_token: TokenFunction,
) -> Result<(), c_int> {
    let slice = unsafe { core::slice::from_raw_parts(p_text as *const c_uchar, n_text as usize) };

    // Map errors to SQLITE_OK because failing here means that the database
    // wouldn't accessible.
    let input = core::str::from_utf8(slice).map_err(|_| SQLITE_OK)?;

    let mut ctx = TokenContext {
        p_ctx,
        x_token,
        normalized: String::with_capacity(1024),
    };

    let mut finder = LinkFinder::new();
    finder.url_must_have_scheme(false);
    finder.kinds(&[LinkKind::Url]);

    for span in finder.spans(input) {
        let token = span.as_str();
        match span.kind() {
            Some(LinkKind::Url) => {
                // Emit scheme
                let url = match memmem::find(token.as_bytes(), b"://") {
                    Some(off) => {
                        on_token(&mut ctx, input, &token[..off])?;
                        &token[off + 3..]
                    }
                    None => token,
                };

                let (host, path) = match memchr3(b'/', b'?', b'#', url.as_bytes()) {
                    Some(off) => (&url[..off], &url[off + 1..]),
                    None => (url, &url[url.len()..]),
                };

                // Emit auth
                let host = match memchr(b'@', host.as_bytes()) {
                    Some(off) => {
                        on_token(&mut ctx, input, &host[..off])?;
                        &host[off + 1..]
                    }
                    None => host,
                };

                // Split off port
                let (host, port) = match memchr(b':', host.as_bytes()) {
                    Some(off) => (&host[..off], &host[off + 1..]),
                    None => (host, &host[host.len()..]),
                };

                let mut last_off: usize = 0;

                let host_range = get_slice_range(input, host);
                if flags & FTS5_TOKENIZE_QUERY == 0 {
                    // Emit host parts: www.youtube.com, youtube.com, com as
                    // synonyms
                    for off in memchr_iter(b'.', host.as_bytes()) {
                        on_raw_token(
                            &mut ctx,
                            &host[last_off..],
                            if last_off == 0 {
                                TokenType::Normal
                            } else {
                                TokenType::Synonym
                            },
                            &host_range,
                        )?;

                        // Note: we intentionally don't emit the tld
                        last_off = off + 1;
                    }
                } else {
                    on_raw_token(&mut ctx, host, TokenType::Normal, &host_range)?;
                }

                // Emit port
                on_token(&mut ctx, input, port)?;

                // Emit path
                on_token(&mut ctx, input, path)?;
            }
            _ => {
                on_token(&mut ctx, input, token)?;
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
            0,
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
                0,
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
                FTS5_TOKENIZE_DOCUMENT,
                vec!["www.example.com", "example.com"],
            ),
            (
                "www.example.com",
                FTS5_TOKENIZE_QUERY,
                vec!["www.example.com"],
            ),
            (
                "example.com?abc",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "abc"],
            ),
            (
                "example.com#abc",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "abc"],
            ),
            (
                "example.com/path#abc",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "path", "abc"],
            ),
            (
                "example.com?abc#def",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "abc", "def"],
            ),
            (
                "example.com?abc/def",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "abc", "def"],
            ),
            (
                "example.com/#def",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "def"],
            ),
            (
                "example.com:123/abc?def",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["example.com", "123", "abc", "def"],
            ),
            (
                "https://www.youtube.com/watch?v=test",
                FTS5_TOKENIZE_DOCUMENT,
                vec![
                    "https",
                    "www.youtube.com",
                    "youtube.com",
                    "watch",
                    "v",
                    "test",
                ],
            ),
            (
                "https://a:b@example.com:1234/",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["https", "a:b", "example.com", "1234"],
            ),
            ("blog.google", FTS5_TOKENIZE_DOCUMENT, vec!["blog.google"]),
            // TODO: ignore known second-level domains when tokenizing
            (
                "amazon.co.uk ",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["amazon.co.uk", "co.uk"],
            ),
            (
                "email@a.b.c.com ",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["email", "a.b.c.com"],
            ),
            (
                "http://example.com/",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["http", "example.com"],
            ),
            (
                "git+ssh://example.com/",
                FTS5_TOKENIZE_DOCUMENT,
                vec!["git", "ssh", "example.com"],
            ),
            ("youtube.com.", FTS5_TOKENIZE_DOCUMENT, vec!["youtube.com"]),
        ];

        for (input, flags, expected_tokens) in test_vectors {
            let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];
            signal_fts5_tokenize_internal(
                &mut tokens as *mut _ as *mut c_void,
                flags,
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

    fn tokenize(input: &str, flags: c_int) -> Vec<(TokenType, String, c_int, c_int)> {
        let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];
        signal_fts5_tokenize_internal(
            &mut tokens as *mut _ as *mut c_void,
            flags,
            input.as_bytes().as_ptr() as *const c_char,
            input.len() as i32,
            token_callback,
        )
        .expect("tokenize internal should not fail");
        tokens
    }

    #[test]
    fn it_tokenizes_urls_with_correct_offsets() {
        let input = "See https://www.signal.org:443/abc/def?q=1##hello/world for details";
        let mut tokens: Vec<(TokenType, String, c_int, c_int)> = vec![];
        signal_fts5_tokenize_internal(
            &mut tokens as *mut _ as *mut c_void,
            0,
            input.as_bytes().as_ptr() as *const c_char,
            input.len() as i32,
            token_callback,
        )
        .expect("tokenize internal should not fail");

        assert_eq!(
            tokens,
            [
                (TokenType::Normal, "see", 0, 3),
                (TokenType::Normal, "https", 4, 9),
                (TokenType::Normal, "www.signal.org", 12, 26),
                (TokenType::Synonym, "signal.org", 12, 26),
                (TokenType::Normal, "443", 27, 30),
                (TokenType::Normal, "abc", 31, 34),
                (TokenType::Normal, "def", 35, 38),
                (TokenType::Normal, "q", 39, 40),
                (TokenType::Normal, "1", 41, 42),
                (TokenType::Normal, "hello", 44, 49),
                (TokenType::Normal, "world", 50, 55),
                (TokenType::Normal, "for", 56, 59),
                (TokenType::Normal, "details", 60, 67),
            ]
            .map(|(token_type, s, start, end)| (token_type, s.to_owned(), start, end))
        );
    }

    #[test]
    fn it_tokenizes_urls_with_userinfo_with_correct_offsets() {
        let input = "See https://user:pw@www.signal.org:443/abc/def?q=1 for details";

        assert_eq!(
            tokenize(input, FTS5_TOKENIZE_DOCUMENT),
            [
                (TokenType::Normal, "see", 0, 3),
                (TokenType::Normal, "https", 4, 9),
                (TokenType::Normal, "user:pw", 12, 19),
                (TokenType::Normal, "www.signal.org", 20, 34),
                (TokenType::Synonym, "signal.org", 20, 34),
                (TokenType::Normal, "443", 35, 38),
                (TokenType::Normal, "abc", 39, 42),
                (TokenType::Normal, "def", 43, 46),
                (TokenType::Normal, "q", 47, 48),
                (TokenType::Normal, "1", 49, 50),
                (TokenType::Normal, "for", 51, 54),
                (TokenType::Normal, "details", 55, 62),
            ]
            .map(|(token_type, s, start, end)| (token_type, s.to_owned(), start, end))
        );

        // Every reported range has to be a valid slice of the input, otherwise
        // fts5's `snippet()`/`highlight()` read out of bounds.
        for (_token_type, token, start, end) in tokenize(input, FTS5_TOKENIZE_DOCUMENT) {
            let (start, end) = (start as usize, end as usize);
            assert!(end <= input.len(), "{token:?} ends past the input");
            assert!(start <= end, "{token:?} starts after its end");
            input.get(start..end).unwrap_or_else(|| {
                panic!("{token:?} range {start}..{end} is not a valid slice of the input")
            });
        }
    }

    #[test]
    fn it_never_reports_offsets_past_the_end_of_the_input() {
        let long_userinfo = format!("t://{}@/common", "a".repeat(2036));
        let inputs = [
            long_userinfo.as_str(),
            "https://user@example.com/common",
            "http://a:b@host.example.com:8080/path?q=1",
            "user@example.com/",
            "user@example.com:8080/path",
            "hello user:pw@example.com/path#frag world",
            "☃@example.com/☃",
            "a@b:1/c",
        ];

        for input in inputs {
            for flags in [FTS5_TOKENIZE_DOCUMENT, FTS5_TOKENIZE_QUERY] {
                for (_token_type, token, start, end) in tokenize(input, flags) {
                    let (start, end) = (start as usize, end as usize);
                    assert!(
                        start <= end && end <= input.len(),
                        "token {token:?} of {input:?} reported {start}..{end}, \
                         which is not within 0..{}",
                        input.len()
                    );
                    input.get(start..end).unwrap_or_else(|| {
                        panic!(
                            "token {token:?} of {input:?} reported {start}..{end}, \
                             which is not a valid slice of the input"
                        )
                    });
                }
            }
        }
    }
}
