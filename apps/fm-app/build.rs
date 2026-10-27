fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../packaging/icon.ico");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../packaging/icon.ico")
            .set("ProductName", fm_brand::APP_NAME)
            .set("FileDescription", fm_brand::APP_NAME)
            .set("CompanyName", fm_brand::ORG_NAME)
            .compile()?;
    }
    Ok(())
}
