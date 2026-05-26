# API Overview

Base URL: `http://localhost:3001`

## Main endpoints

- `POST /runs`
- `GET /runs/:id`
- `GET /runs/:id/execution`
- `GET /runs/:id/execution/stream`
- `GET /runs/:id/world`
- `POST /runs/:id/approvals/:channel`
- `POST /runs/:id/resume`
- `POST /runs/:id/cancel`

## Which one matters most?

If you are building an operator-facing client, start with:

- `/runs` to create work
- `/runs/:id/execution` for current runtime state
- `/runs/:id/execution/stream` for live updates

That is the shared run model behind the product surfaces.

`POST /runs` accepts `tool_scopes`; those scopes bind the run writ to built-in
tools and any programmable capabilities loaded on the server.
