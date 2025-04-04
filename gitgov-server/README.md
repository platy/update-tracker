# Govdiff-rs

The main binary runs a webserver which exposes the history stored in a repository and watches for incoming update emails as a trigger to retrieve updated pages and add them to the repository.
Govdiff expects to find update emails on the filesystem, you can achieve this by installing and running [smtp-dump](https://crates.io/crates/smtp-dump).

## Repository

You'll need a repository ready for storing and reading file histories. You can initialise a new one or clone an existing one.

### Clone

```sh
ssh njk.onl "cd /mnt/govdiff/repo && tar czf - url" | tar xzf - -C ./repo/
```

### Init

```sh
mkdir -p ./repo/url/www.gov.uk ./repo/tag
```

## Run

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
Fetch updates             Median time per request   33ms
Fetch updates             Average time per request  34ms
Fetch updates             Sample standard deviation 6ms

Fetch Brexit updates      Total requests            100
Fetch Brexit updates      Successful requests       100
Fetch Brexit updates      Failed requests           0
Fetch Brexit updates      Median time per request   85ms
Fetch Brexit updates      Average time per request  85ms
Fetch Brexit updates      Sample standard deviation 10ms

Fetch update              Total requests            100
Fetch update              Successful requests       100
Fetch update              Failed requests           0
Fetch update              Median time per request   10ms
Fetch update              Average time per request  10ms
Fetch update              Sample standard deviation 1ms

Fetch large update        Total requests            100
Fetch large update        Successful requests       100
Fetch large update        Failed requests           0
Fetch large update        Median time per request   16ms
Fetch large update        Average time per request  18ms
Fetch large update        Sample standard deviation 5ms
```

The index page and the smaller documents are fine, but the larger documents take too long, it's likely because they allocate a lot of memory, this memory usage is leading to OOMKills on k8s and this will likely happen on publishing if actual users happen to look at large docs like this one which I actually want to link to. Allocations on large documents are clearly a problem as running this benchmark throws usage up by a couple of hundred megabytes

Solved the issue with the memory usage on diffs by caching diff results, they won't be invalid until I change the algorithm anyway.

## Add another subscription

Use a new @govdiff.njk.onl email address to make the subscription. Then Get access to the updates repo, look in the outbox (assuming update-tracker has already processed the confirmation email). Find the email, extract the link, then de-SMTP it by removing the =CRLF line endings and unescape equals signs (escaped as =3D)

## Process access logs

Get the logs from the server, strip out the lines which aren't about the access logs using:

```regex
s/^[^2].*\n//gm
```

Then convert to csv with:
```regex
s/> (\S*) +(\S*) +< (.*) \( *(\S*)ms\) <- +(.*) + (\/.*) \[Referer: (".*") User-agent: (".*")\]/$1,$2,$3,$4,$5,$6,$7,$8/g
```
