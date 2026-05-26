# Getting Started

## 1. Start the backend runtime

```bash
git clone https://github.com/gryszzz/OpenThymos.git
cd OpenThymos/thymos
cargo run -p thymos-server
```

Default server URL: `http://localhost:3001`

Optional programmable capabilities:

```bash
THYMOS_TOOL_MANIFEST_DIRS=../tools cargo run -p thymos-server
```

## 2. Choose your interface

### Web console

```bash
cd ..
npm install
npm run dev
```

Open `http://localhost:3000/runs`

### CLI

```bash
cd thymos
cargo run -p thymos-cli -- run "Inspect the repo and explain Thymos" --provider mock --follow
```

### VS Code

Build the extension in `thymos/clients/vscode`, launch it in Extension Development Host, and point it at `http://localhost:3001`.

## 3. Understand the flow

Every run goes through:

`Intent -> Proposal -> Commit`

That flow is the same no matter which interface you use.
