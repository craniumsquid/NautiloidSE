fn main() {
    let a: Vec<String> = std::env::args().collect();
    let names: Vec<&str> = a[3..].iter().map(|s| s.as_str()).collect();
    let n = nautiloid_save_editor::lspk::extract(
        std::path::Path::new(&a[1]),
        &names,
        std::path::Path::new(&a[2]),
    )
    .unwrap();
    println!("{n} extracted");
}
