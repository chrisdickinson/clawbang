#!/bin/bash

clawbang <<EOF
fn main() {
  println!("hello world!");

  std::process::exit(32)
}
EOF

