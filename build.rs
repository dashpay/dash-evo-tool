fn main() {
    // Windows-specific icon embedding
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/DET_LOGO.ico");
        res.compile().unwrap();
    }
}
