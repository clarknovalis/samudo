# samudo-api

The wire types of the HTTP API of
[samudo](https://github.com/clarknovalis/samudo), a self-hosted task
manager: serde types only, without logic or I/O. In code the crate is
`samu_api`.

```rust
use samu_api::{ApiError, ErrorCode};

let body = r#"{"error": "not_found", "message": "no such object"}"#;
let answer: ApiError = serde_json::from_str(body).unwrap();
assert_eq!(answer.error, ErrorCode::NotFound);
```

- The routes are described in
  [`docs/api.md`](https://github.com/clarknovalis/samudo/blob/main/docs/api.md).
- Every type has JSON examples under `tests/fixtures/`, checked in both
  directions. They are the contract.
- A field a reader does not know is skipped, an enum value it does not
  know reads as `unknown`. Removing or reinterpreting a field is a new
  major version; the version is independent of samudo's own.

Licensed under AGPL-3.0-or-later, MIT or Apache-2.0, at your option.
