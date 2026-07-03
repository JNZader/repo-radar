# repo-radar

Read this in: [English](README.md) · [Español](README.es.md)

**Motor de descubrimiento de repositorios de GitHub alimentado por feeds, para encontrar ideas que vale la pena robar a propósito.**

[![CI](https://github.com/JNZader/repo-radar/actions/workflows/ci.yml/badge.svg)](https://github.com/JNZader/repo-radar/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

repo-radar agrega repositorios desde feeds RSS/Atom, GitHub Trending, HackerNews, Reddit y el descubrimiento de GitHub Skills, los enriquece con metadata de GitHub, los categoriza y cruza los hallazgos contra tus propios repositorios para hacer aflorar ideas accionables. Se distribuye como un único binario de CLI (`repo-radar`) con un dashboard web opcional en Axum.

- **Versión:** 0.1.0 · **Edición:** Rust 2024 · **Licencia:** MIT
- **Runtime async:** Tokio (multi-thread)
- **HTTP:** `reqwest` (rustls) · **API de GitHub:** `octocrab` · **Feeds:** `feed-rs`

## Panorama

- Escrito en Rust, con un flujo CLI-first y un dashboard web opcional.
- Corre un pipeline de descubrimiento: fetch → dedupe → filter → categorize → analyze → cross-reference → report.
- Ingesta múltiples tipos de fuentes, cachea en disco las respuestas de la API de GitHub y soporta autenticación opcional del dashboard por bearer token.
- Acepta tanto el bloque de config legado `[[feeds]]` como el nuevo bloque multi-fuente `[[sources]]`.
- Usa una arquitectura de puertos y adaptadores (hexagonal) para mantener aisladas las fuentes, filtros, analizadores, reporters y adaptadores web.

## Inicio Rápido

```sh
cargo install --path .
repo-radar config init
export REPO_RADAR_GITHUB_TOKEN="ghp_your_token_here"
export REPO_RADAR_GITHUB_USERNAME="your-github-username"
repo-radar scan
```

`repo-radar config init` escribe un `config.toml` comentado en la ruta de config XDG. Si corrés `scan` antes de que exista una config, imprime un mensaje de primera ejecución en vez de fallar.

## Tabla de Contenidos

- [Comandos de la CLI](#comandos-de-la-cli)
- [Flags globales](#flags-globales)
- [Configuración](#configuración)
- [Variables de entorno](#variables-de-entorno)
- [Dashboard](#dashboard)
- [Tipos de fuente](#tipos-de-fuente)
- [Pipeline](#pipeline)
- [Arquitectura](#arquitectura)
- [Desarrollo](#desarrollo)
- [Licencia](#licencia)

## Comandos de la CLI

Subcomandos disponibles: `scan`, `report`, `ideas`, `diff`, `compare`, `serve`, `config`.

### `scan`

Corre el pipeline de descubrimiento y cachea los resultados para su uso posterior por `report`, `ideas` y `diff`.

```sh
repo-radar scan
repo-radar scan --dry-run          # imprime la config resuelta y sale sin hacer fetch
repo-radar scan --accumulate       # además corre el pipeline de KB, guardando candidatos en SQLite
repo-radar scan --kb-path ./kb.sqlite
```

Flags:

- `--dry-run` — imprime la configuración completamente resuelta y sale; sin llamadas de red.
- `--accumulate` — después del filtrado, corre el pipeline de knowledge base y persiste los candidatos en la KB SQLite.
- `--kb-path <PATH>` — sobrescribe la ruta SQLite de la KB (clave de config `kb.db_path`).
- `--stage <STAGE>` / `--backfill` — aceptados por el parser pero **todavía no implementados**; por ahora son no-ops.

### `report`

Regenera un reporte desde el scan cacheado más reciente (sin volver a hacer fetch).

```sh
repo-radar report
repo-radar report --format json     # markdown (default) | json | console
repo-radar report --output ./my-reports
```

### `ideas`

Extrae ideas accionables desde los resultados de scan cacheados (o desde un archivo JSON explícito).

```sh
repo-radar ideas
repo-radar ideas --input results.json
repo-radar ideas --min-relevance 0.5   # default: 0.1
repo-radar ideas --print               # además imprime las ideas en la consola
```

Sin `--input`, se usa el `report-*.json` más reciente del directorio de salida del reporter.

### `diff`

Compara dos snapshots de scan cacheados y muestra los repositorios nuevos, eliminados y modificados.

```sh
repo-radar diff                                  # penúltimo vs último
repo-radar diff --scan-a <scan-id> --scan-b <scan-id>
```

Los IDs de scan provienen de los scans cacheados en el directorio de datos. Sin IDs, `diff` compara el penúltimo scan contra el último. Requiere al menos dos scans guardados.

### `compare`

Compara un repositorio externo contra uno propio y genera ideas. Este camino usa un gateway LLM (ver la config `[kb]`) y un caché de knowledge base.

```sh
repo-radar compare --source rust-lang/rustlings --target ~/code/my-project
repo-radar compare --source https://github.com/owner/repo --target https://github.com/me/project --output ideas.md
```

- `--source` — repo externo a estudiar (URL de GitHub o atajo `owner/repo`).
- `--target` — tu repo a mejorar (ruta local o URL de GitHub). Los targets locales se analizan vía export de `repoforge`.
- `--force` — vuelve a analizar aunque los repos ya estén cacheados en la KB.
- `--output <FILE>` — además escribe las ideas generadas en un archivo markdown.

### `serve`

Levanta el dashboard web.

```sh
repo-radar serve
repo-radar serve --port 8080                    # default: 3000
repo-radar serve --host 0.0.0.0 --port 3000     # host default: 127.0.0.1
```

### `config`

```sh
repo-radar config init     # crea una config default en la ruta XDG (no-op si ya existe)
repo-radar config show     # imprime la configuración completamente resuelta como TOML
```

## Flags Globales

Aplican a todos los subcomandos:

```sh
repo-radar --config ./custom-config.toml scan
repo-radar -v scan
repo-radar -vv scan
```

- `--config, -c <PATH>` — usa un archivo de config específico en vez del default XDG.
- `-v` — logging debug. `-vv` — logging trace. (Se puede sobrescribir con el env filter `RUST_LOG`.)

## Configuración

El archivo de config vive por default en `$XDG_CONFIG_HOME/repo-radar/config.toml` (creado por `config init`). El bloque de abajo documenta todas las claves soportadas; el template generado es un punto de partida comentado más chico.

```toml
[general]
data_dir = "~/.local/share/repo-radar"   # default: el directorio de datos XDG
log_level = "info"
backfill_batch_size = 50

[filter]
min_stars = 10
languages = ["Rust", "TypeScript"]
topics = ["cli", "htmx"]
exclude_forks = true
exclude_archived = true

[analyzer]
# repoforge_path = "/path/to/repoforge"
timeout_secs = 60
# llm_model = "openai/gpt-4o-mini"
deep_analysis_top_n = 5
deep_analysis_min_relevance = 0.15

[crossref]
own_repos = ["./exports/my-repo.json"]

[reporter]
output_dir = "./output"
format = "markdown"   # markdown | json | console

[cache]
ttl_secs = 86400
rate_limit_threshold = 100
# cache_dir = "./cache"   # default: data_dir/cache

[kb]
enabled = false
db_path = "kb.db"
llm_gateway_url = "http://localhost:3456"
llm_model = "openai/gpt-4o-mini"
# llm_auth_token = "sk-..."

# Sintaxis legada solo-RSS (todavía soportada por compatibilidad).
[[feeds]]
url = "https://example.com/feed.xml"
name = "Example Feed"

# Sintaxis multi-fuente preferida.
[[sources]]
type = "rss"
url = "https://example.com/feed.xml"
name = "Example Feed"

[[sources]]
type = "github_trending"
language = "Rust"
since = "daily"   # daily | weekly | monthly

[[sources]]
type = "hackernews"
limit = 30

[[sources]]
type = "reddit"
subreddits = ["rust", "programming"]
limit = 25

[[sources]]
type = "github_skills"
limit = 30
```

### Notas de configuración

- `[[feeds]]` es la sintaxis legada solo-RSS y sigue soportada. `[[sources]]` es la sintaxis preferida para tipos de fuente mixtos. Ambas pueden convivir en un mismo archivo.
- Los valores de config se validan al cargar: URLs de feed/fuente inválidas, formatos de reporter desconocidos, timeouts y tamaños de batch fuera de rango, y niveles de log desconocidos se reportan todos juntos. Los tokens de GitHub sospechosos o de placeholder generan una advertencia.
- `REPO_RADAR_GITHUB_USERNAME` y `REPO_RADAR_GITHUB_TOKEN` se pueden omitir si la CLI de GitHub (`gh`) está instalada y autenticada — repo-radar recurre a `gh auth token` y `gh api user` para resolverlos.
- El pipeline de KB corre automáticamente cuando `[kb] enabled = true`, o de forma ad hoc con `repo-radar scan --accumulate`.

## Variables de Entorno

| Variable | Propósito |
|---|---|
| `REPO_RADAR_GITHUB_TOKEN` | Token de la API de GitHub para enriquecimiento de metadata, filtrado y cross-reference. Recurre a `gh auth token` si no está seteada. |
| `REPO_RADAR_GITHUB_USERNAME` | Usuario de GitHub para matchear los hallazgos contra tus propios repositorios. Recurre a `gh api user` si no está seteada. |
| `REPO_RADAR_DASHBOARD_TOKEN` | Bearer token opcional que, cuando está seteado, protege todas las rutas del dashboard. |
| `REPO_RADAR_LLM_API_KEY` | API key opcional para LLM. Se carga en la config y se muestra (enmascarada) en la página de config del dashboard. Nota: las llamadas LLM de `compare`/KB se autentican con `[kb] llm_auth_token`, no con esta variable. |
| `RUST_LOG` | Env filter estándar de `tracing`; sobrescribe el nivel de verbosidad `-v`/`-vv`. |

## Dashboard

El comando `serve` levanta un dashboard construido con Axum, HTMX y templates de Askama; los gráficos se renderizan con Chart.js (cargado desde una CDN, así que necesitan acceso a la red). Provee:

- Una página de resumen con desgloses por categoría y por lenguaje, y gráficos.
- Scans disparados desde el navegador con progreso en tiempo real vía Server-Sent Events (`/api/scan/events`).
- Vistas de comparación de repositorios (`/compare/{owner}/{repo}`).
- Navegación del historial de scans y detalle por reporte (`/reports`, `/reports/{id}`).
- Vistas de diff entre scans (`/diff`, `/diff/{id_a}/{id_b}`).
- Una página de inspección de config con los secretos enmascarados.

Por default el servidor escucha en `127.0.0.1:3000`. Usá `repo-radar serve --host 0.0.0.0` para setups de LAN, contenedor o reverse-proxy, y seteá `REPO_RADAR_DASHBOARD_TOKEN` para exigir un bearer token en cada ruta.

## Tipos de Fuente

| Fuente | Descripción | Clave de config |
|---|---|---|
| RSS/Atom | Cualquier feed que enlace a repositorios de GitHub | `type = "rss"` |
| GitHub Trending | Repositorios trending por lenguaje y período (`daily`/`weekly`/`monthly`) | `type = "github_trending"` |
| HackerNews | Historias con enlaces a GitHub | `type = "hackernews"` |
| Reddit | Posts de los subreddits configurados que contienen enlaces a GitHub | `type = "reddit"` |
| GitHub Skills | Repositorios que contienen archivos `SKILL.md` | `type = "github_skills"` |

Múltiples fuentes se ejecutan concurrentemente y se combinan; una sola fuente configurada se usa directamente, y sin fuentes se recurre a un no-op.

## Pipeline

El pipeline de `scan` corre en secuencia:

1. **Fetch** — trae las entradas de todas las fuentes configuradas.
2. **Dedupe** — descarta los repositorios ya vistos (registrados en `seen.json`).
3. **Filter** — aplica criterios de metadata de GitHub (stars, lenguaje, topics, forks, estado archivado). Requiere un token de GitHub; de lo contrario esta etapa es un no-op.
4. **Categorize** — asigna categorías a partir de keywords y topics.
5. **Analyze** — extrae resúmenes, features y pistas del stack técnico. Usa `repoforge` cuando `analyzer.repoforge_path` está seteado; de lo contrario, un analizador no-op.
6. **Cross-reference** — matchea los hallazgos contra tus propios repositorios. Requiere un usuario de GitHub; de lo contrario, un no-op.
7. **Report** — escribe los resultados en el formato configurado.

Los resultados se cachean en el directorio de datos para que `report`, `ideas` y `diff` puedan correr sin volver a hacer fetch.

## Arquitectura

repo-radar sigue una arquitectura de puertos y adaptadores (hexagonal).

| Ruta | Rol |
|---|---|
| `src/domain/` | Traits y modelos centrales para fuentes, filtros, categorizadores, analizadores, cross-reference, scoring y reporting |
| `src/adapters/` | Implementaciones concretas: RSS, GitHub Trending, HackerNews, Reddit, GitHub Skills, reporters, KB y el dashboard web |
| `src/infra/` | Caché, seen-store, persistencia de scans, KB SQLite, rate limiting y manejo de errores |
| `src/pipeline.rs` | Orquesta el pipeline de descubrimiento |
| `src/kb_pipeline.rs` | Construye el pipeline opcional de acumulación de knowledge base |
| `src/config.rs` | Carga de config TOML, overlays de variables de entorno y validación |

Cada etapa del pipeline está basada en traits, con adaptadores no-op en cada frontera para poder intercambiar o testear las etapas de forma aislada.

## Desarrollo

```sh
cargo build            # build debug
cargo test             # corre la suite de tests unitarios + de integración
cargo clippy           # lints
cargo run -- scan      # correr desde el código fuente
```

Se incluyen un `Dockerfile` y un `docker-compose.dev.yml` para correr en contenedores, y el CI está definido en `.github/workflows/ci.yml`.

## Licencia

MIT. (El archivo `LICENSE` está referenciado por el badge de arriba; agregá uno en la raíz del repositorio si falta.)
