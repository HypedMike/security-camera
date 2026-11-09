// The mini app (dist/) is embedded in the binary: rebuild when it changes.
fn main() {
    println!("cargo:rerun-if-changed=dist");
}
