# Gitgov-rs

gitgov expects to find update emails on the filesystem, you can acheive this by installing and running [smtp-dump](https://crates.io/crates/smtp-dump).

run as daemon:

```sh
date >> logs; bash -c 'setsid cargo run </dev/null &>>logs & jobs -p %1'
```

check daemon:
```sh
lsof logs
```

stop daemon:
```sh
killall gitgov
```

restart:
```sh
cargo build; killall update-tracker; date >> logs; bash -c 'setsid cargo run </dev/null &>>logs & jobs -p %1'
```


## Benchmarks 

Some basic benchmarking, before I'm tempted to work on performance.

Memory usage and startup time is dependent on the number of tags.

Serving http is the main thing to worry about, and it's not fast, but I'd probably rather see that there is a need for it to be faster before chaning anything specifically. as it's static html, the load time is still pretty good, the only worry is if there are a lot of concurrent requests.

Run a benchmark on it's own:

```sh
drill --benchmark benchmark.yaml --stats
```

Or, set release mode to include debug signals, in the root Cargo.toml:
```toml
[profile.release]
debug = 1
```

And run a heap profile with the benchmark:

```sh
cargo run --features dhat-heap --release
```

```
Fetch updates             Total requests            100
Fetch updates             Successful requests       100
Fetch updates             Failed requests           0
Fetch updates             Median time per request   35ms
Fetch updates             Average time per request  36ms
Fetch updates             Sample standard deviation 4ms

Fetch update              Total requests            100
Fetch update              Successful requests       100
Fetch update              Failed requests           0
Fetch update              Median time per request   11ms
Fetch update              Average time per request  12ms
Fetch update              Sample standard deviation 3ms

Fetch living in germany update Total requests            100
Fetch living in germany update Successful requests       100
Fetch living in germany update Failed requests           0
Fetch living in germany update Median time per request   1333ms
Fetch living in germany update Average time per request  1354ms
Fetch living in germany update Sample standard deviation 126ms

Time taken for tests      35.3 seconds
Total requests            300
Successful requests       300
Failed requests           0
Requests per second       8.51 [#/sec]
Median time per request   35ms
Average time per request  467ms
Sample standard deviation 631ms
```

The index page and the smaller documents are fine, but the larger documents take too long, it's likely because they allocate a lot of memory, this memory usage is leading to OOMKills on k8s and this will likely happen on publishing if actual users happen to look at large docs like this one which I actually want to link to. Allocations on large documents are clearly a problem as running this benchmark throws usage up by a couple of hundred megabytes
