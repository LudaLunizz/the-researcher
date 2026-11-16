Closes #

## Summary

## Testing

- [ ] `cargo fmt --check` and `cargo clippy --locked --all-targets -- -D warnings`
- [ ] `cargo test --locked` (includes the Rust/Python parity test)
- [ ] `python3 -I -m unittest discover -s tests -v`
- [ ] `npm test` in `worker/` (only when `worker/` changed)
- [ ] Tried on a real system (say which; the popup is not covered by CI)
