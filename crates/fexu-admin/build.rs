//! The admin UI is embedded from the source tree. Node.js is not invoked.

fn main() {
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=static");
}
