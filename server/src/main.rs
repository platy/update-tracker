use std::thread;

use crate::data::Data;

mod data;
mod web;
mod gitgov;

const LISTEN_ADDR: &str = "localhost:8080";

fn main() {
    println!("Loading data");

    let data = Data::new();

    thread::spawn(gitgov::run);

    web::listen(LISTEN_ADDR, data);
}
