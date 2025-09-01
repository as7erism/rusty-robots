# rusty robots

backend server implementation of [ricochet robots](https://en.wikipedia.org/wiki/Ricochet_Robots)
written in rust bundled with a svelte frontend

## development

### with frontend

```bash
# install frontend dependencies
cd client
npm install

# run server
cd ..
cargo run
```

### without frontend

```bash
cargo run --no-default-features
```
