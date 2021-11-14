use std::{
    sync::{Arc, RwLock},
    thread,
};

use update_tracker::{data::Data, ingress, web};

fn main() {
    let new_repo_path = dotenv::var("NEW_REPO").unwrap();
    println!("Loading data");

    let data = Arc::new(RwLock::new(Data::load(new_repo_path.as_ref())));
    let data2 = data.clone();

    thread::spawn(move || {
        if let Err(err) = ingress::run(new_repo_path.as_ref(), data2) {
            println!("Ingress failed : {} {:?}", err, err);
        }
    });

    web::listen(dotenv::var("LISTEN_ADDR").as_deref().unwrap_or("127.0.0.1:8080"), data);
}
