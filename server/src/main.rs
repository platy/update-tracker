fn main() {
    const LISTEN_ADDR: &str = "localhost:8080";
    println!("Listen on http://{}", LISTEN_ADDR);

    rouille::start_server_with_pool(LISTEN_ADDR, None, |request| {
        rouille::match_assets(&request, "./static")
    });
}
