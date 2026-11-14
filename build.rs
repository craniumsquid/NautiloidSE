//! Gives the Windows program its icon. Does nothing on other systems.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon/icon.ico");
    #[cfg(windows)]
    {
        // Only when the program being built is for Windows too.
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut resource = winresource::WindowsResource::new();
            resource.set_icon("assets/icon/icon.ico");
            resource
                .compile()
                .expect("the icon could not be put into the program");
        }
    }
}
