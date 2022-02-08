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

```sh
cargo install drill
drill --benchmark benchmark.yaml --stats
```

```
Fetch updates             Total requests            100
Fetch updates             Successful requests       100
Fetch updates             Failed requests           0
Fetch updates             Median time per request   38ms
Fetch updates             Average time per request  38ms
Fetch updates             Sample standard deviation 5ms

Fetch update              Total requests            100
Fetch update              Successful requests       100
Fetch update              Failed requests           0
Fetch update              Median time per request   12ms
Fetch update              Average time per request  12ms
Fetch update              Sample standard deviation 2ms

Time taken for tests      1.3 seconds
Total requests            200
Successful requests       200
Failed requests           0
Requests per second       154.91 [#/sec]
Median time per request   26ms
Average time per request  25ms
Sample standard deviation 14ms
```

It's not fast but it should handle a handful of users easily.
