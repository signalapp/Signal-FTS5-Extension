# Overview

Signal-FTS5-Extension is a C ABI library which exposes a
[FTS5](https://www.sqlite.org/fts5.html) tokenizer function named
`signal_tokenizer` that:

- Segments UTF-8 strings into words according to
  [Unicode standard](http://www.unicode.org/reports/tr29/)
- Normalizes and removes diacritics from words
- Converts words to lower case

When used as a custom FTS5 tokenizer this enables application to support CJK
symbols in full-text search.

# Extension Build/Usage Example

```sh
cargo build --features extension --release
```

Load extension from `./target/release/libsignal_tokenizer`.

```sql
CREATE VIRTUAL TABLE
fts
USING fts5(content, tokenize='signal_tokenizer')
```

# Generating headers

```sh
cbindgen --profile release . -o target/release/fts5-tokenizer.h
```

# Legal things

## License

Copyright 2023 Signal Messenger, LLC.

Licensed under the AGPLv3: http://www.gnu.org/licenses/agpl-3.0.html
