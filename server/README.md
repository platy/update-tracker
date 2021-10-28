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
cargo build; killall gitgov; date >> logs; bash -c 'setsid cargo run </dev/null &>>logs & jobs -p %1'
```
