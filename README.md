# clawbang

A command line interface meant to bridge the gap between Rust and shell
scripting. Intended for use with HEREDOCs and shebangs:

```bash
$ clawbang <<EOF
  fn main() {
    println!("hello world!")
  }
EOF
```

Or as a binary executable file:

```
#!/usr/bin/env clawbang

fn main() {
  println!("hello world!")
}
```

---

## performance

Clawbang takes the file input, hashes it, and then looks for a cached compiled
copy. If no copy is present, Clawbang compiles your code and caches it by that
hash for later lookup.

If your program _failed to compile_, those error messages are cached as well.

## cargo build options and dependencies

Specify `Cargo.toml` values in frontmatter:

```
#!/usr/bin/env clawbang
+++
[dependencies]
foo = "1.2.0"
+++

fn main() {
  println!("hello world!")
}
```

## license

MIT
