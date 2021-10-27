use crate::data::Data;

mod data;
mod web;

const LISTEN_ADDR: &str = "localhost:8080";

fn main() {
    println!("Loading data");

    let data = Data::new();

    web::listen(LISTEN_ADDR, data);
}
