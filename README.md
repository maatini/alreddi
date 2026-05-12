# Alreddi — Edeka Retail Integration Layer

**Hybride BFF- & CQRS-Middleware** für die Edeka-Systemlandschaft.
Verbindet heterogene Backends (PIM, Preis-Engines, SAP LUNAR) mit
unterschiedlichen Konsumenten (Picnic, Mobile Apps, Chatbots, POS/Kasse)
über zwei getrennte Pfade mit klar definierten SLAs.

> **Alreddi** = _already_ + _REDDI_. Die Daten sind bereits im Cache,
> bevor die Kasse fragt — `< 15 ms` p99 für den POS-Lese-Pfad.

[![Rust](https://img.shields.io/badge/rust-1.95%2B-orange?style=flat&logo=rust)](https://www.rust-lang.org)
[![Devbox](https://img.shields.io/badge/devbox-0.17-blue?style=flat&logo=nixos)](https://www.jetify.com/devbox)
[![Status](https://img.shields.io/badge/status-Phase%201%20complete-green?style=flat)](#phasen--roadmap)
[![License](https://img.shields.io/badge/license-proprietary-red?style=flat)](#lizenz)

![Alreddi Architecture](alreddi.png)

---

## Architektur

```
                          ┌──────────────────────────────┐
                          │         API Consumer          │
                          │  Picnic · Apps · POS · Chat   │
                          └──────────────┬───────────────┘
                                         │
                          ┌──────────────▼───────────────┐
                          │      Alreddi Gateway          │
                          │    (Rust · Axum · Tower)      │
                          │                              │
                          │  ┌────────────────────────┐  │
                          │  │  Governance Layer       │  │
                          │  │  APQ → Cost → Coalesce  │  │
                          │  └────────────────────────┘  │
                          │                              │
                          │  ┌──────────┐ ┌────────────┐ │
                          │  │  Pfad A  │ │  Pfad B    │ │
                          │  │ Federation│ │  CQRS      │ │
                          │  │ (GraphQL) │ │ (REST)     │ │
                          │  └────┬─────┘ └─────┬──────┘ │
                          └───────┼─────────────┼────────┘
                                  │             │
                    ┌─────────────┼─────┐  ┌────▼──────────┐
                    │             │     │  │  REDDI Event  │
                    │  PIM   Price  LUNAR│  │  Bus (Solace) │
                    │             │     │  └───────┬───────┘
                    └─────────────┼─────┘          │
                                  │         ┌──────▼───────┐
                          GraphQL │         │  POS Cache    │
                        Subgraphs │         │  (In-Memory)  │
                                  │         └──────────────┘
```

### Pfad A — GraphQL Federation (zustandslos)

Für flexible Clients (Picnic, Apps, Chatbots). Apollo Federation v2 mit
`@key(fields: "ean")` für Entity-Resolution.

- **PimSubgraph** — Produktstammdaten, Kategorien, Marken
- **PriceSubgraph** — Preisinformationen, Währungen
- **LunarSubgraph** (Phase 3) — SAP-ERP-Anbindung via ID-Translation

### Pfad B — CQRS Fast-Read (zustandsbehaftet)

Für latenzkritische Clients (POS/Kasse, Self-Checkout). Asynchrone
Event-Konsumption aus REDDI (Solace), lokaler In-Memory-Cache,
keine externen HTTP-Calls im Request-Pfad.

**SLA: `< 15 ms` p99 pro Lese-Operation.**

---

## Governance-Mechanismen

Alle Interceptoren laufen im Gateway-Prozess als Tower-Middleware vor
der Schema-Ausführung:

| Mechanismus | Beschreibung | Fehlerfall |
|---|---|---|
| **Query Cost Analysis** | AST-basierte statische Kostenanalyse vor jeder Query-Ausführung | `HTTP 429` |
| **Request Coalescing** | Single-Flight: identische parallele Queries werden zu einem Backend-Call gebündelt | Timeout → eigener Call |
| **Automated Persisted Queries** | SHA-256-Cache für Queries; Client sendet Hash statt Query-Text | `PERSISTED_QUERY_NOT_FOUND` |
| **ID-Translation** | Whitelisting + Mapping EAN/GTIN → SAP MATNR vor Subgraph-Routing (Phase 3) | — |
| **Strukturiertes Logging** | JSON nach stdout/stderr, kein externes SDK | — |

### Cost Model

| Pattern | Kostenpunkte |
|---|---|
| Skalarfeld | 1 |
| Objektfeld (leaf) | 2 |
| Listen-Einstiegspunkte (`articles`, `search`) | 5 |
| Connection-Felder (`edges`) | 3 |
| Introspection (`__typename`, `__schema`) | 0 |
| Verschachtelungstiefe > `max_depth` | `429` |

Konfigurierbar via Umgebungsvariablen: `GATEWAY_MAX_COST` (default: 100),
`GATEWAY_MAX_DEPTH` (default: 10).

---

## Projektstruktur

```
graphql-federation/
├── gateway/                        # Rust Gateway Binary
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                 # Server-Bootstrap, Axum-Router, Handler
│       ├── config.rs               # Umgebungsvariablen-Konfiguration
│       ├── logging.rs              # JSON-Logging (tracing-subscriber)
│       ├── cost_analysis.rs        # Query Cost Analyzer (ca. 370 LOC)
│       ├── coalescing.rs           # Single-Flight Coalescer (DashMap + broadcast)
│       ├── apq.rs                  # APQ-Cache (SHA-256, LRU-Eviction)
│       └── schema.rs               # Federation-Schema + Mock-Resolvern
├── supergraph/
│   ├── supergraph.graphqls         # Komponiertes Supergraph-Schema (Federation v2)
│   ├── pim.graphqls                # PIM-Subgraph
│   └── price.graphqls              # Preis-Subgraph
├── devbox.json                     # Devbox-Container (Rust 1.95, graphqurl)
├── devbox.lock
├── .gitignore
└── CLAUDE.md                       # Projektinstruktionen
```

---

## Quickstart

### Voraussetzungen

- [Devbox](https://www.jetify.com/devbox) installiert
- Alternativ: Rust 1.95+ (stable), Cargo

### Build & Test

```bash
# Devbox-Shell starten
devbox shell

# Build
cargo build --manifest-path gateway/Cargo.toml

# Tests (27 Unit-Tests)
cargo test --manifest-path gateway/Cargo.toml

# Release-Build
cargo build --manifest-path gateway/Cargo.toml --release
```

### Server starten

```bash
# Standard (Port 4000)
cargo run --manifest-path gateway/Cargo.toml

# Mit Konfiguration
GATEWAY_PORT=8080 GATEWAY_MAX_COST=200 cargo run --manifest-path gateway/Cargo.toml
```

### GraphQL-Queries testen

```bash
# Health Check
curl http://localhost:4000/health

# Artikel per EAN
curl -s -X POST http://localhost:4000/graphql \
  -H 'Content-Type: application/json' \
  -d '{"query": "{ article(ean: \"4012345678901\") { ean name brand price { amount currency } category { name } } }"}'

# Suche
curl -s -X POST http://localhost:4000/graphql \
  -H 'Content-Type: application/json' \
  -d '{"query": "{ search(query: \"EDEKA\", first: 3) { totalCount edges { node { ean name } } } }"}'

# Cost-Limit testen (wird mit 429 abgelehnt)
curl -s -X POST http://localhost:4000/graphql \
  -H 'Content-Type: application/json' \
  -d '{"query": "{ a1:article(ean:\"1\"){ean} a2:article(ean:\"2\"){ean} a3:article(ean:\"3\"){ean} a4:article(ean:\"4\"){ean name brand category{id name}price{amount currency}} }"}'

# APQ: nur Hash senden (erwartet PERSISTED_QUERY_NOT_FOUND beim ersten Mal)
QUERY_HASH=$(echo -n '{ article(ean: "4012345678901") { ean name } }' | shasum -a 256 | cut -d' ' -f1)
curl -s -X POST http://localhost:4000/graphql \
  -H 'Content-Type: application/json' \
  -d "{\"extensions\": {\"persistedQuery\": {\"sha256Hash\": \"$QUERY_HASH\", \"version\": 1}}}"
```

### Mit graphqurl (gq)

```bash
# Via devbox
gq http://localhost:4000/graphql -q '{ article(ean: "4012345678901") { ean name brand } }'
```

---

## Umgebungsvariablen

| Variable | Default | Beschreibung |
|---|---|---|
| `GATEWAY_HOST` | `0.0.0.0` | Bind-Adresse |
| `GATEWAY_PORT` | `4000` | Port |
| `GATEWAY_MAX_COST` | `100` | Max. erlaubte AST-Kosten pro Query |
| `GATEWAY_MAX_DEPTH` | `10` | Max. erlaubte Selektionstiefe |
| `GATEWAY_APQ_CACHE_SIZE` | `10000` | Max. Anzahl gecachter APQ-Queries |
| `GATEWAY_COALESCING_ENABLED` | `true` | Single-Flight-Coalescing aktiv |
| `GATEWAY_REQUEST_TIMEOUT_SECS` | `30` | Coalescing-Timeout |
| `RUST_LOG` | `info` | Log-Level (tracing env-filter) |

---

## Phasen & Roadmap

| Phase | Status | Inhalt |
|---|---|---|
| **Phase 1** | ✅ abgeschlossen | Gateway-Scaffold, Governance-Interceptoren, Supergraph-Schema, Mock-Resolvern |
| **Phase 2** | 🔜 nächste | CQRS POS Fast-Read Service: REDDI-Event-Ingestion, In-Memory-Cache, `GET /api/v1/pos/article/{ean}` |
| **Phase 3** | geplant | Subgraph-Resolver, ID-Translation (EAN → MATNR), LUNAR-Anbindung |

---

## Nicht-Funktionale Anforderungen

- **POS-Latenz:** `< 15 ms` p99 (Pfad B, Phase 2)
- **Kein Datadog-SDK:** Ausschließlich JSON-Logging nach stdout/stderr
- **Keine DB-Zugriffe im synchronen POS-Pfad**
- **Explizite Timeouts:** Jede IO-Operation mit Context-Cancellation (Phase 2)
- **Typensicherheit:** Vollständig typisiertes Rust, keine `any`-Typen

---

## Lizenz

Proprietär — Edeka Zentrale AG & Co. KG
