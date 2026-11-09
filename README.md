### Dr Driving PC
a port of dr driving made to run on machines other than android.

<img width="1920" height="1080" alt="2026-11-08_08-18-56" src="https://github.com/user-attachments/assets/3024d773-4650-4b7a-b979-55bb4f76bc92" />
<img width="1920" height="1080" alt="2026-11-08_08-17-32" src="https://github.com/user-attachments/assets/25268686-1329-492a-8041-90d556ffef05" />


### Installation
download the binaries for your operating system from the [releases](https://github.com/KrishnaSSH/drdriving-pc/releases/latest) 

### Build from Source
make sure your system has rust toolchain installed
you can install it from [rustup.rs](https://rustup.rs)

```
cargo run --release -p drdriving
```

### Game data files

##### Linux
```
$XDG_DATA_HOME/drdriving/save.json
```
will use
```
~/.local/share/drdriving/save.json
```

if that variable isn't set


##### Windows
```
 %APPDATA%\drdriving\save.json
```

##### MacOS

```
~/Library/Application Support/drdriving/save.json
```



### Discord Rich Presence

`DRDRIVING_NO_RPC=1` to turn off the discord rpc.
