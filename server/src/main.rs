use std::thread;

use crate::data::Data;

mod data;
mod ingress;
mod web;

const LISTEN_ADDR: &str = "localhost:8080";

fn main() {
    println!("Loading data");

    let data = Data::new();

    thread::spawn(ingress::run);

    web::listen(LISTEN_ADDR, data);
}
