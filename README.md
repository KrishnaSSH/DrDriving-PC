### Dr Driving PC
a port of dr driving made to run on machines other than android.


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


