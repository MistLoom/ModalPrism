fn main() {
    if let Err(e) = ability_sdk_codegen::generate_default() {
        panic!("ability codegen: {e}");
    }
}
