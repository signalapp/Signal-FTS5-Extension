//
// Copyright 2023 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use libc::{c_char, c_int, c_void};

pub struct Fts5Tokenizer {}

// sqlite3.h
pub const SQLITE_OK: c_int = 0;
pub const SQLITE_INTERNAL: c_int = 2;
pub const SQLITE_MISUSE: c_int = 21;

pub type TokenFunction = extern "C" fn(
    p_ctx: *mut c_void,
    t_flags: c_int,
    p_token: *const c_char,
    n_token: c_int,
    i_start: c_int,
    i_end: c_int,
) -> c_int;
