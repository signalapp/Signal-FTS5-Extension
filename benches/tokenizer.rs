//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use core::ptr::null_mut;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use libc::{c_char, c_int, c_void};
use signal_tokenizer::{signal_fts5_tokenize, Fts5Tokenizer, SQLITE_OK};

extern "C" fn noop_callback(
    _ctx: *mut c_void,
    _flags: c_int,
    _token: *const c_char,
    _token_len: c_int,
    _start: c_int,
    _end: c_int,
) -> c_int {
    return SQLITE_OK;
}

fn tokenize(input: &str) {
    signal_fts5_tokenize(
        &mut Fts5Tokenizer {},
        null_mut(),
        0,
        input.as_bytes().as_ptr() as *const c_char,
        input.len() as i32,
        noop_callback,
    );
}

fn fts5_benchmark(c: &mut Criterion) {
    let latin_lower_60kb = "hello ".repeat(10 * 1024);
    let latin_upper_60kb = "HELLO ".repeat(10 * 1024);
    let diacritics_60kb = "öplö ".repeat(10 * 1024);
    let cjk_60kb = "你好".repeat(10 * 1024);

    c.bench_function("tokenize latin lowercase 60kb", |b| {
        return b.iter(|| tokenize(black_box(&latin_lower_60kb)));
    });

    c.bench_function("tokenize latin uppercase 60kb", |b| {
        return b.iter(|| tokenize(black_box(&latin_upper_60kb)));
    });

    c.bench_function("tokenize diacritics 60kb", |b| {
        return b.iter(|| tokenize(black_box(&diacritics_60kb)));
    });

    c.bench_function("tokenize cjk 60kb", |b| {
        return b.iter(|| tokenize(black_box(&cjk_60kb)));
    });
}

criterion_group!(benches, fts5_benchmark);
criterion_main!(benches);
