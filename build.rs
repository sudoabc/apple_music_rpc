fn main() {
    // Asigning icon to the windows executable file
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/apple_music.ico");
        res.compile().unwrap();
    }
}