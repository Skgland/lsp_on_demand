# lsp_on_demand

A Programm that listens for incoming connections and for each
connection starts a language server and passes through the connection.

## Installation via cargo

```shell
cargo install --git https://github.com/Skgland/lsp_on_demand.git
```

## Configuration

Some options can be configured using environment variables:

| Variable       | Default                                               | Description              |
|:---------------|:------------------------------------------------------|:-------------------------|
| `JAVA_PATH`    | `java`                                                | the java binary to run   |
| `LSP_JAR_PATH` | `./server/kieler-language-server.{linux,osx,win}.jar` | the lsp jar to use       |
