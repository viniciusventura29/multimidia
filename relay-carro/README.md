# relay-carro

Dá à central do carro um endereço público. É um Worker do Cloudflare com um
Durable Object (`Carro`) na **conta pessoal** (`account_id` no
`wrangler.jsonc`), porque o wrangler deste Mac também enxerga a da empresa.

- `POST /mcp/<chave>`: servidor MCP (Streamable HTTP, sem sessão). As
  ferramentas são as que o carro anunciou ao conectar, mais `carro_estado`.
  A lista sai mesmo com o carro desligado (a última conhecida); a chamada é
  que responde "desconectado".
- `GET /carro`: WebSocket do carro (header `x-carro-chave`, `x-carro-app`).
- `PUT /apk/<chave>`: repassa um APK em pedaços ao Mãos, que instala.
- `PUT /link/<chave>` e `GET /m/<código>`: link curto da primeira instalação.

```sh
wrangler secret put CHAVE     # a mesma de ~/.eclipse-carro.properties
wrangler deploy
wrangler dev                  # local; a chave vem de .dev.vars (CHAVE="…")
wrangler types                # antes do tsc: gera worker-configuration.d.ts
```

Nada é guardado além da lista de ferramentas, de quando o carro foi visto e do
APK do link (2 h). O R2 não está ligado nesta conta, e não faz falta: o APK
grande atravessa o relay direto para o carro.
