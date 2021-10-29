use std::{
    sync::{Arc, RwLock},
    thread,
};

use update_tracker::{data::Data, ingress, web};

const LISTEN_ADDR: &str = "0.0.0.0:80";

fn main() {
    println!("Loading data");

    let data = Arc::new(RwLock::new(Data::load()));
    let data2 = data.clone();

    thread::spawn(|| ingress::run(data2));

    web::listen(LISTEN_ADDR, data);
}
