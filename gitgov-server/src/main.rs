#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use std::{
    sync::{Arc, RwLock},
    thread,
};

use gitgov_server::{data::Data, ingress, web};

fn main() {
    #[cfg(feature = "dhat-heap")]
    let profiler = dhat::Profiler::builder().file_name("dhat-heap-setup.json").build();

    let new_repo_path = dotenv::var("NEW_REPO").unwrap();
    println!("Loading data");

    let data = Arc::new(RwLock::new(Data::load(new_repo_path.as_ref())));
    let data2 = data.clone();

    thread::spawn(move || {
        if let Err(err) = ingress::run(new_repo_path.as_ref(), data2) {
            println!("Ingress failed : {} {:?}", err, err);
        }
    });

    #[cfg(feature = "dhat-heap")]
    drop(profiler);
    #[cfg(feature = "dhat-heap")]
    thread::spawn(move || {
        let profiler = dhat::Profiler::builder().file_name("dhat-heap-drill.json").build();
        println!("Profiling dhat allocations");
        std::process::Command::new("drill")
            .args(["--benchmark", "benchmark.yaml", "--stats"])
            .spawn()
            .unwrap()
            .wait()
            .unwrap();
        println!("Exiting to write dhat allocations");
        drop(profiler);
        std::process::exit(0);
    });

    web::listen(dotenv::var("LISTEN_ADDR").as_deref().unwrap_or("127.0.0.1:8080"), data);
}
