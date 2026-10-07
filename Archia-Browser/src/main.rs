use archia_browser::Browser;

fn main() {
    let browser = Browser::new();
    println!("Archia Browser {}", browser.version());
}
