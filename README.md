Building an MCP Server Hosting Platform (AnyContext-Like) – Full-Stack Guide

Introduction

AnyContext Overview: AnyContext is a cloud platform for hosting Model Context Protocol (MCP) servers – specialized connectors that let AI agents (LLM-based assistants, chatbots, etc.) interact with external systems through a standardized interface . In an AnyContext-style architecture, AI agents (MCP clients) communicate with various back-end services (databases, SaaS APIs, etc.) via MCP servers, decoupling the AI from specific integrations. The MCP server translates an LLM’s request into external API calls or database queries, then returns results so the LLM can incorporate that information . This “universal adapter” approach (MCP is an open standard by Anthropic) makes it easy to plug in new tools or data sources without changing core AI logic .

Architecture of an MCP hosting platform (inspired by AnyContext). An MCP client (AI assistant) connects via the MCP protocol to the platform’s control plane, which orchestrates multiple MCP servers (PostgreSQL connector, Slack connector, OpenAPI connector, etc.). Each MCP server runs in isolation (e.g. a container) and connects to its external system (database, API) on behalf of the AI .

What We’re Building: We’ll create a full-stack guide to implement a similar service, with a Rust backend and Next.js frontend. The stack includes:
	•	Backend: Rust (using a modern web framework, e.g. Axum or Actix for high-performance async APIs ), PostgreSQL for persistent storage, and robust authentication (JWT-based tokens in HTTP-only cookies for security, or session-based as an alternative).
	•	Frontend: Next.js (React) application, styled with Tailwind CSS for a responsive UI, allowing users to register/login, manage their MCP servers, initiate context sessions, and monitor usage metrics in real-time.

We’ll cover database schema design, backend API implementation (with code samples in Rust), containerizing and orchestrating MCP server processes, and a Next.js frontend with modern UI patterns. Security, scalability, and maintainability best practices will be highlighted throughout.

Database Schema Design (PostgreSQL)

To support users, MCP server deployments, context sessions, and usage tracking, we design a relational schema in PostgreSQL. Below is a possible schema (with SQL DDL statements to create tables):

-- Users table: stores registered user accounts
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- MCP Servers table: each record is an MCP server deployment
CREATE TABLE mcp_servers (
    id SERIAL PRIMARY KEY,
    owner_id INTEGER NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,         -- human-friendly name or identifier
    server_type TEXT NOT NULL,  -- type of MCP server (e.g. "PostgreSQL", "Slack")
    config JSONB,              -- stored configuration (API keys, endpoints, etc.)
    status TEXT NOT NULL,       -- e.g. "creating", "running", "stopped"
    api_key TEXT NOT NULL,      -- secret key for accessing this server (for MCP clients)
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Context Sessions table: tracks sessions or conversations using an MCP server
CREATE TABLE context_sessions (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    started_at TIMESTAMPTZ DEFAULT NOW(),
    ended_at TIMESTAMPTZ           -- null if session is ongoing
    -- Additional fields like 'description' or 'session_token' could be included
);

-- Usage Metrics table: logs usage events or aggregates for MCP servers
CREATE TABLE usage_metrics (
    id SERIAL PRIMARY KEY,
    server_id INTEGER NOT NULL REFERENCES mcp_servers(id),
    timestamp TIMESTAMPTZ DEFAULT NOW(),
    event_type TEXT,        -- e.g. "request", "error", etc.
    details JSONB           -- optional JSON details (payload size, duration, etc.)
);

Design Notes: We use foreign keys to link mcp_servers to the owning users, and sessions/metrics to their server. The mcp_servers.config is a JSONB field to flexibly store server-specific config (like API credentials or settings). We also include an api_key for each server – a secret token that an AI client must supply (e.g. via an X-API-Key header) when connecting to that MCP server  . This provides an extra layer of access control per deployment.

The usage_metrics table can function as an event log for each server (one row per operation or request) or store aggregated stats (e.g. daily usage counts), depending on how we implement metrics collection. For simplicity, we’ll log each significant event for now. This data will enable both historical analysis (e.g. usage over time) and real-time monitoring of activity.

We will use a database migration tool or ORM to create these tables. For example, if using Diesel (a type-safe Rust ORM), a migration file would contain SQL as above and can be run with diesel migration run  to set up the schema. If using an async SQL library like SQLx or Prisma, similar table definitions would be applied in the setup phase.

Backend Setup (Rust, Axum Framework)

Project Initialization: Start a new Rust project (binary crate) for the backend. In Cargo.toml, add dependencies for our web server, database, and auth needs. For example:

[dependencies]
axum = "0.6"              # Web framework for routing and handlers
tokio = { version = "1.28", features = ["full"] }  # Async runtime
serde = { version = "1.0", features = ["derive"] } # For JSON serialization
serde_json = "1.0"
dotenvy = "0.15"          # To load env vars (like DATABASE_URL)
sqlx = { version = "0.6", features = ["postgres", "runtime-tokio-native-tls"] } 
# or alternatively: diesel = { version = "2.1.0", features = ["postgres"] }

jsonwebtoken = "8.2"      # For JWT creation/verification
argon2 = "0.4"            # For secure password hashing (Argon2id)
uuid = "1.3"              # To generate unique IDs (optional, e.g. for session tokens)
tracing = "0.1"           # For logging

Why Axum? Axum (from the Tokio project) provides a fast, modular HTTP server with strong async support and type-safe routing . It integrates nicely with Tower middleware and Rust’s ecosystem. We choose Axum for clarity and performance, but Actix-Web could also be used with similar principles. We’ll use PostgreSQL via an async driver (SQLx) or Diesel with a connection pool for database operations.

Configuration: Load configuration from environment variables or a .env file (using dotenvy). Critical settings include the DATABASE_URL (Postgres connection string), a JWT signing secret key, and possibly a Docker host or other runtime configs. We’ll also enable logging with tracing for debugging.

Database Connection Pool: Initialize a connection pool to Postgres at startup. For SQLx, for example:

use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();  // load .env if present
    let database_url = std::env::var("DATABASE_URL")?;
    // Create a connection pool (with 5 connections here)
    let db_pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url).await?;
    // ... set up Axum router next
}

This pool (db_pool) will be added to Axum’s application state so handlers can access the DB. In Axum, we can use an extractor for state or a global using Extension. For example:

use axum::{Router, Extension};

let app = Router::new()
    .route("/", axum::routing::get(root_handler))
    // ... (other routes)
    .layer(Extension(db_pool));  // make pool available to all handlers

Now, any handler can accept an Extension<PgPool> argument to get the pool.

Authentication: User Registration and JWT Login

Password Hashing: Storing passwords in plaintext is a huge security risk. We use Argon2id (the recommended Argon2 variant for password hashing)  to securely hash user passwords before saving in the database. Argon2id is memory-hard and resistant to GPU cracking and side-channel attacks, making it ideal for password security  .

In Rust, the argon2 crate (part of RustCrypto) can be used along with rand_core for salt generation. For example, to hash a password:

use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use rand_core::OsRng;

fn hash_password(plain: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    // Configure Argon2 with default params (Argon2id)
    let argon2 = Argon2::default();
    let password_hash = argon2.hash_password(plain.as_bytes(), &salt)?;
    Ok(password_hash.to_string())  // store the hash string (encoded with salt)
}

When a user registers, we’ll call hash_password on their plaintext password and save the resulting hash (which encodes the salt and algorithm parameters) in the users.password_hash column.

User Registration Endpoint: We create an Axum handler for POST /api/register that accepts JSON {"email": "...", "password": "..."}. It will:
	1.	Validate the input (ensure email isn’t already used, password meets criteria).
	2.	Hash the password.
	3.	Insert a new user row into the database.

Using SQLx with query macros, for example:

use axum::{Json, http::StatusCode};
use serde::Deserialize;

#[derive(Deserialize)]
struct RegisterRequest { email: String, password: String }

async fn register_user(
    Extension(pool): Extension<PgPool>,
    Json(payload): Json<RegisterRequest>
) -> Result<StatusCode, (StatusCode, String)> {
    // Basic validation:
    if payload.password.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "Password too short".into()));
    }
    // Hash the password
    let hashed = hash_password(&payload.password)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Hashing failed: {}", e)))?;
    // Insert user (assuming `users(email, password_hash)` as per schema)
    let result = sqlx::query!(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2)",
        payload.email,
        hashed
    )
    .execute(&pool).await;
    match result {
        Ok(_) => Ok(StatusCode::CREATED),
        Err(sqlx::Error::Database(db_err)) if db_err.constraint() == Some("users_email_key") => {
            // Unique constraint violation on email
            Err((StatusCode::CONFLICT, "Email already registered".into()))
        }
        Err(e) => {
            tracing::error!("DB insert error: {:?}", e);
            Err((StatusCode::INTERNAL_SERVER_ERROR, "Server error".into()))
        }
    }
}

This handler returns an HTTP 201 Created on success, or appropriate errors (400 for invalid data, 409 if email exists, etc.). Password handling is secure: even if the database is compromised, the raw passwords aren’t exposed.

Login Endpoint (JWT generation): The POST /api/login handler will verify credentials and issue a JWT. Steps:
	1.	Fetch the user by the provided email.
	2.	Use Argon2 to verify the provided password against the stored password_hash.
	3.	If valid, create a JWT token signed with our secret, including user ID and maybe email in the claims.
	4.	Set the JWT as a cookie (HttpOnly, Secure) or return it in response JSON (depending on the auth strategy).

Using the jsonwebtoken crate, define our claims structure and create a token:

use jsonwebtoken::{encode, Header, EncodingKey};
use serde::Serialize;
use axum::response::{IntoResponse, Response};

#[derive(Serialize)]
struct Claims {
    sub: i32,          // user id
    exp: usize         // expiration (timestamp)
    // (add other claims as needed, e.g., email, issued-at, etc.)
}

async fn login_user(
    Extension(pool): Extension<PgPool>,
    Json(payload): Json<LoginRequest>   // similar to RegisterRequest
) -> Result<Response, (StatusCode, String)> {
    let user = sqlx::query!("SELECT id, password_hash FROM users WHERE email = $1", payload.email)
        .fetch_optional(&pool).await
        .map_err(|e| {
            tracing::error!("DB query error: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Server error".into())
        })?;
    let user = user.ok_or((StatusCode::UNAUTHORIZED, "Invalid email or password".into()))?;
    // Verify password
    let parsed_hash = argon2::PasswordHash::new(&user.password_hash)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Server error".into()))?;
    if Argon2::default().verify_password(payload.password.as_bytes(), &parsed_hash).is_err() {
        return Err((StatusCode::UNAUTHORIZED, "Invalid email or password".into()));
    }
    // Password correct – create JWT
    let expiration = chrono::Utc::now()
        .checked_add(chrono::Duration::hours(24))
        .expect("valid timestamp")
        .timestamp() as usize;
    let claims = Claims { sub: user.id, exp: expiration };
    let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET not set");
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| {
            tracing::error!("JWT encode error: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Token error".into())
        })?;
    // Set as secure HttpOnly cookie
    let cookie = format!("auth_token={token}; HttpOnly; Secure; SameSite=Strict; Path=/");
    let mut res = Response::new("Login successful".into());
    res.headers_mut().append(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    Ok(res)
}

Here we return a Response with a Set-Cookie header containing the JWT. We use the HttpOnly flag so that JavaScript on the frontend cannot read the token, mitigating XSS risks . The Secure flag ensures it’s only sent over HTTPS. (If we were building a pure SPA without cookies, we could send the JWT in the JSON response and store it in memory or localStorage, but cookies with HttpOnly are generally safer for persistent storage of tokens  .)

JWT verification middleware: Protected API routes (like those managing servers or sessions) should require a valid JWT. In Axum, we can implement this as an extractor or middleware. For example, define an extractor that checks the Authorization: Bearer <token> header or cookie and, if valid, yields the user’s information:

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use jsonwebtoken::{decode, DecodingKey, Validation};

struct AuthUser { user_id: i32 }  // data extracted from token

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // Try to get token from cookie or Authorization header
        let token = if let Some(cookie_header) = parts.headers.get(axum::http::header::COOKIE) {
            // find auth_token cookie
            let cookies = cookie_header.to_str().unwrap_or("");
            cookies.split(';')
                   .find_map(|c| {
                       let c = c.trim();
                       c.starts_with("auth_token=")
                           .then(|| c.trim_start_matches("auth_token=").to_string())
                   })
        } else if let Some(authz) = parts.headers.get(axum::http::header::AUTHORIZATION) {
            authz.to_str().ok()
                .and_then(|s| s.strip_prefix("Bearer "))
                .map(|s| s.to_owned())
        } else {
            None
        };
        let token = token.ok_or((StatusCode::UNAUTHORIZED, "Missing token".into()))?;
        let secret = std::env::var("JWT_SECRET").unwrap();
        let decoded = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default()
        ).map_err(|_| (StatusCode::UNAUTHORIZED, "Invalid token".into()))?;
        Ok(AuthUser { user_id: decoded.claims.sub })
    }
}

Now a handler can require an AuthUser extractor to automatically enforce auth. For example:

async fn list_servers(Extension(pool): Extension<PgPool>, AuthUser { user_id }: AuthUser)
    -> Result<Json<Vec<McpServerSummary>>, (StatusCode, String)>
{
    // user_id is authenticated
    let recs = sqlx::query!("SELECT id, name, server_type, status, created_at 
                              FROM mcp_servers WHERE owner_id = $1", user_id)
        .fetch_all(&pool).await
        .map_err(|e| { tracing::error!("DB error: {:?}", e); (StatusCode::INTERNAL_SERVER_ERROR, "DB error".into()) })?;
    let servers = recs.into_iter().map(|r| McpServerSummary {
        id: r.id, name: r.name, server_type: r.server_type,
        status: r.status, created_at: r.created_at
    }).collect();
    Ok(Json(servers))
}

This will only execute if the JWT is present and valid, otherwise Axum will return a 401 before entering the handler. We’d apply similar extraction on all routes that need authentication. By structuring it this way, our endpoints remain mostly stateless (JWTs carry the user info), which is good for scalability.

MCP Server Lifecycle Management API

One core feature of the platform is allowing users to create and manage MCP servers (the connectors). We will expose RESTful endpoints to handle the lifecycle:
	•	Create a new server deployment – e.g. POST /api/servers with JSON details.
	•	List my servers – GET /api/servers (as shown above).
	•	Get server details – GET /api/servers/{id}.
	•	Start/Stop/Delete server – e.g. POST /api/servers/{id}/start, POST /api/servers/{id}/stop, DELETE /api/servers/{id}.
	•	Configure server – possibly covered by create or separate update calls.
	•	Context sessions – POST /api/servers/{id}/sessions to start a session (if needed), GET /api/servers/{id}/sessions to list sessions.
	•	Usage metrics – GET /api/servers/{id}/metrics to get usage data (optionally with query params for timeframe), and maybe GET /api/servers/{id}/metrics/stream for real-time streaming of metrics.

Let’s walk through creating a server deployment, as it is the most involved. The POST /api/servers endpoint will allow a user to deploy a new MCP server. The request might include: a name for the deployment, a server_type (if the platform offers a catalog of server types, like “PostgreSQL”, “Slack”, “WeatherAPI”, etc.), and any configuration data (credentials or parameters).

DB Insertion: When this API is called, we first insert a row into mcp_servers with status “creating”. We generate a unique api_key for this server (a random UUID or token) which the client will use to connect. For example:

#[derive(Deserialize)]
struct CreateServerRequest {
    name: String,
    server_type: String,
    config: Option<serde_json::Value>
}

async fn create_server(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id }: AuthUser,
    Json(payload): Json<CreateServerRequest>
) -> Result<Json<ServerInfo>, (StatusCode, String)> {
    // Basic validations
    if payload.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Name is required".into()));
    }
    // (We might also validate server_type against allowed types)
    let api_key = uuid::Uuid::new_v4().to_string();  // generate a random token
    // Insert new server record
    let rec = sqlx::query!(
        "INSERT INTO mcp_servers (owner_id, name, server_type, config, status, api_key)
         VALUES ($1, $2, $3, $4, 'creating', $5)
         RETURNING id, status, created_at",
        user_id, payload.name, payload.server_type, payload.config, api_key
    )
    .fetch_one(&pool).await
    .map_err(|e| {
        tracing::error!("DB insert error: {:?}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create server".into())
    })?;
    let server_id = rec.id;
    // Launch the server in background (see below)
    spawn_server_task(server_id, payload.server_type.clone(), payload.config.clone(), api_key.clone(), pool.clone());
    // Return initial info to client
    let info = ServerInfo {
        id: server_id,
        name: payload.name,
        server_type: payload.server_type,
        status: rec.status,
        api_key,
        created_at: rec.created_at
    };
    Ok(Json(info))
}

Here ServerInfo is a struct to serialize back basic info (we include the api_key so the user can copy it for configuring their AI agent). Note how we call spawn_server_task(...) after inserting the DB record – this is where we handle the actual container launch asynchronously.

Spawning the MCP server process: We don’t want to block the HTTP request while, say, pulling a Docker image or initializing a server. Instead, we spawn a background task (Tokio task) to handle it. Rust’s async runtime allows spawning tasks that run concurrently on the thread pool without blocking the main thread  . This means we can return a response immediately (perhaps indicating the server is being set up) and do the heavy lifting in the background.

A simplified example of spawn_server_task using Docker (via the Bollard crate) could look like:

use bollard::Docker;
use bollard::container::{CreateContainerOptions, Config as ContainerConfig, StartContainerOptions};
use bollard::models::HostConfig;

fn spawn_server_task(server_id: i32, server_type: String, config: Option<serde_json::Value>, api_key: String, pool: PgPool) {
    tokio::spawn(async move {
        let docker = Docker::connect_with_local_defaults().unwrap();
        // Map server_type to a Docker image name:
        let image = match server_type.as_str() {
            "PostgreSQL" => "ghcr.io/anycontext/postgres-mcp:latest",
            "Slack" => "ghcr.io/anycontext/slack-mcp:latest",
            // ... other mappings ...
            _ => "ghcr.io/anycontext/default-mcp:latest"
        };
        // Prepare container create options
        let container_name = format!("mcp-server-{}", server_id);
        let create_opts = CreateContainerOptions { name: container_name.as_str(), platform: None };
        // Set environment variables for the container (e.g., API key, config)
        let mut env_vars = vec![ format!("MCP_API_KEY={}", api_key) ];
        if let Some(cfg) = &config {
            // Pass each config field as an env var, or mount a config file as needed
            if let Some(obj) = cfg.as_object() {
                for (key, value) in obj {
                    env_vars.push(format!("CFG_{}={}", key.to_uppercase(), value));
                }
            }
        }
        let container_config = ContainerConfig {
            image: Some(image),
            env: Some(env_vars),
            host_config: Some(HostConfig {
                auto_remove: Some(true),  // remove container on stop
                // potentially port mappings if needed
                ..Default::default()
            }),
            ..Default::default()
        };
        // Create container
        match docker.create_container(Some(create_opts), container_config).await {
            Ok(container) => {
                // Start container
                if let Err(e) = docker.start_container(&container.id, None::<StartContainerOptions<String>>).await {
                    tracing::error!("Failed to start container {}: {:?}", container.id, e);
                    sqlx::query!("UPDATE mcp_servers SET status = 'error' WHERE id = $1", server_id)
                        .execute(&pool).await.ok();
                    return;
                }
                // Update status to running
                sqlx::query!("UPDATE mcp_servers SET status = 'running' WHERE id = $1", server_id)
                    .execute(&pool).await.ok();
                tracing::info!("MCP server {} ({}) is now running", server_id, server_type);
            }
            Err(e) => {
                tracing::error!("Container creation failed for server {}: {:?}", server_id, e);
                sqlx::query!("UPDATE mcp_servers SET status = 'error' WHERE id = $1", server_id)
                    .execute(&pool).await.ok();
            }
        }
    });
}

Using Bollard, we connect to the local Docker daemon and call the Docker API to create and start a container. The example above maps environment variables and uses an auto_remove config so containers don’t pile up after stop. The sequence is: create then start the container (as per Docker API usage)  . We then update the database status to “running” on success, or “error” on failure.

Note: In a production setting, you might want more robust error handling (retries, image pull if not present, etc.) and possibly not auto-remove containers so you can inspect them if needed. Also, mapping network ports or using an overlay network might be necessary to route requests to the container. For simplicity, we assume the container images handle MCP communication (likely via an HTTP server or SSE on a known port) and perhaps the platform uses a reverse proxy or subdomain (like <id>.mcp.anycontext.io) to forward traffic to the correct container. Implementation of that networking is environment-specific (could use Docker’s internal IP + a proxy, or host-port mapping).

Stopping/Deleting servers: For a stop, we’d call docker.stop_container(container_name, Some(StopContainerOptions{ t: 30 })) (with a 30-second timeout) . Since we named containers with mcp-server-{id}, we can track the name or container ID in the DB (we might add a column for container_id if needed). After stopping, update status to “stopped”. For deletion (if a user wants to terminate a deployment), we could stop the container (if running) and remove the DB entry (or mark it deleted). Removing the container via Docker API (docker.remove_container) frees resources .

Access Control: It’s crucial that all these endpoints check the authenticated user’s rights. For example, if user A tries to stop user B’s server by hitting /api/servers/123/stop, our handler should verify that server 123’s owner_id matches the auth user and return 403 Forbidden if not. This can be done with an extra DB query to fetch the server’s owner for such operations.

Context Sessions: Depending on the MCP server’s nature, a “context session” might represent a persistent conversation or usage context. For example, if an AI is having an extended interaction with a database MCP server, we could log a session start and end. The POST /api/servers/{id}/sessions might create a new row in context_sessions with started_at and return a session_id (and possibly a token or any initialization needed). When the AI finishes using it, we could call PATCH /api/servers/{id}/sessions/{session_id} to mark ended_at.

This aspect can be optional – not all use-cases require explicit session management – but it’s included for completeness. Sessions could be used to group usage metrics or apply per-session limits.

Usage Metrics Collection: Each MCP server (running in its container) might not automatically inform the platform of usage. We have a few options to collect metrics:
	•	Proxy Approach: Route all MCP traffic through a central proxy in the control plane that logs requests. For instance, the platform could host an endpoint (or subdomain) that forwards requests to the container and can increment counters or logs in usage_metrics. This requires implementing an HTTP reverse proxy or similar in the Rust backend or at the web server level (Nginx, etc.).
	•	Server Callbacks: Instrument the MCP server code (if we control it) to send an HTTP callback to the platform for each request it handles. The platform could provide an internal API like POST /api/servers/{id}/metrics that the server calls with details (this could be authenticated by the api_key).
	•	Polling/Logs: In some cases, reading container logs or stats (via Docker API or scraping) is possible, but less real-time.

For simplicity, assume we implement the proxy approach: e.g., the platform issues the MCP server a unique base URL and all AI agent calls go through the platform (with the platform injecting the x-api-key and routing). This way, our Rust code can intercept and log the events.

Regardless of method, once metrics are being recorded in the usage_metrics table, we provide:
	•	GET /api/servers/{id}/metrics?range=24h (for example) to get aggregated stats (like number of requests in last 24h, etc.).
	•	GET /api/servers/{id}/metrics/stream to get live updates.

For the real-time stream, Server-Sent Events (SSE) is a straightforward approach. Axum supports SSE natively, allowing us to push events to the client over a long-lived HTTP connection  . For example:

use axum::response::{sse::Event, sse::Sse};
use futures::Stream;
use std::time::Duration;
use tokio_stream::{StreamExt, wrappers::IntervalStream};

// Simplest example: stream a periodic tick with current count
async fn metrics_stream(
    Extension(pool): Extension<PgPool>,
    AuthUser { user_id }: AuthUser,
    axum::extract::Path(server_id): axum::extract::Path<i32>
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    // (Verify ownership of server_id by user_id first...)
    // Create an async stream that yields an Event every second with latest metrics
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
        .then(move |_| {
            // query the database for the latest metric (or aggregate)
            let count = /* ... fetch count of events last minute ... */
            let data = format!("{{\"req_per_minute\": {count} }}");
            futures::future::ready(Ok(Event::default().data(data)))
        });
    Ok(Sse::new(stream))
}

This is a placeholder example where every second the server would send a JSON string of some metric. In practice, you’d tailor it to push relevant updates (or even push an event when a new usage entry is added, by notifying via channels). The key point is that SSE provides a unidirectional stream ideal for live dashboard updates without constant polling .

Containerization and Orchestration of MCP Servers

Each MCP server is packaged as a microservice. Containerization with Docker is a natural choice to isolate dependencies and environments for each server type. In our design, the platform will use pre-built Docker images for each supported MCP server type (for example, an image for a Postgres connector, an image for Slack connector, etc.). This matches AnyContext’s model where you select an integration and it deploys the corresponding server.

Docker Images: Ensure you have Docker images available for all MCP server types your platform offers. These images should expose an HTTP endpoint (or SSE endpoint) implementing the MCP protocol. For instance, a PostgreSQL MCP Server image might contain a small web service that listens for MCP JSON-RPC requests from the AI and executes SQL queries on a target database . The image might expect certain environment variables like DB_CONNECTION_STRING and the MCP_API_KEY to secure access. We won’t delve into the image implementation here, but we assume they exist.

Launching Containers: We used the Bollard crate to programmatically manage Docker containers from Rust. Bollard is an async Docker client that exposes all Docker API functions (container create/start/stop, image pull, etc.) . Alternatively, one could use Docker CLI via std::process::Command calls, but that’s less efficient and harder to monitor. With Bollard, for example, creating and starting a container is done with a few calls as shown earlier. The snippet we showed follows Docker best practices: create then start a container (the created container is like a configured instance of an image, ready to run)  . We also configured auto_remove so Docker cleans it up after exit, which simplifies cleanup.

One challenge is networking – how do AI agents reach these containers? In AnyContext, deployments are exposed at URLs like https://<deployment-id>.mcp.anycontext.io/sse  . Likely, AnyContext’s control plane sets up a subdomain per server that routes to the container’s internal address. Implementing this might involve running a reverse proxy (like Traefik or Nginx) that dynamically updates routes when new containers start. For our guide, a simpler approach is to have all containers listen on a unique port on the host (or use Docker’s port mapping) and tell the user that URL. For example, on server creation, choose a free port, start the container with HostConfig.port_bindings to bind container’s port (say 8000) to host 0.0.0.0:$PORT, and then construct the URL as https://your-platform-domain:$PORT. This is simpler but not scalable for many users or beyond development (and requires open ports). A production-grade solution should use a proper proxy with subdomains (and valid TLS).

Orchestration & Scalability: Using Docker allows horizontal scaling – you can run many containers on one host or even across a cluster. In a real-world scenario, you might integrate with Kubernetes or a container service (AWS ECS, Google Cloud Run, etc.) to handle scheduling and scaling. For example, there are guides on deploying MCP servers to cloud platforms effortlessly  . In our Rust backend, we could swap out the local Docker calls with calls to a Kubernetes API or cloud SDK to launch containers. The concept remains: the control plane triggers deployments and keeps track of them.

It’s also wise to implement a reaper or monitor service: something that periodically checks on running containers and updates statuses (in case a container crashed, we should mark it as stopped/error and maybe notify the user). Docker allows subscribing to events or simply querying container status.

Additionally, consider resource limits – when creating containers via the API, one can set CPU/memory limits in the HostConfig. This prevents a single user’s server from hogging all resources on the host. In multi-tenant environments, security settings like user namespaces and network isolation are also important (e.g., running containers with a non-root user, limiting capabilities).

Next.js Frontend – User Interface for MCP Platform

On the frontend, we build a Next.js application to provide a user-friendly interface. The app will allow users to:
	•	Register and Login: Provide sign-up and sign-in forms, interacting with our Rust API.
	•	Dashboard of MCP Servers: Display the list of servers the user has deployed, their status, and allow actions (view details, start/stop, create new).
	•	Create New Server: A form or modal to choose a server type from available options, provide a name and necessary config, and trigger creation.
	•	View Context Sessions: If applicable, list sessions or allow starting a new session (this could be integrated into server details, e.g. a “Use in Playground” button).
	•	View Usage Metrics: Show usage statistics for each server, possibly in charts or tables. Real-time metrics might be shown on a server’s detail page or a central dashboard using live updates.

Tech Stack: We use React with Next.js, and Tailwind CSS for styling. Tailwind’s utility classes expedite responsive design and ensure consistency. Next.js gives us an easy way to create pages and API routes (though for API we’ll call the Rust backend, not use Next’s API layer, except maybe for some proxy if needed).

Setting up Next.js & Tailwind: Initialize the project with npx create-next-app@latest (choose TypeScript if comfortable). During setup, you can opt-in to Tailwind integration (Next 13+ can ask “Would you like to use Tailwind CSS? Yes”). This will generate a tailwind.config.js and import Tailwind in globals.css. Confirm Tailwind is working by using some utility class and seeing it styled.

Tailwind is mobile-first, meaning by default styles apply to small screens, and you add responsive prefixes like md: for larger breakpoints  . It’s good practice to design the mobile layout first and enhance for bigger screens . Use flexible units and utility classes (w-full, flexbox, grid) to create fluid layouts that adapt to different devices .

Auth Pages (Register/Login): Create a page for register (e.g. pages/register.tsx) and login (pages/login.tsx). These pages will likely be simple forms. Using React hooks for form state:

// pages/login.tsx
import { FormEvent, useState } from 'react';
import Router from 'next/router';

export default function Login() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError("");
    try {
      const res = await fetch(`${process.env.NEXT_PUBLIC_API_URL}/api/login`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, password }),
        credentials: 'include'  // include cookies in request/response
      });
      if (res.ok) {
        // If using cookie-based auth, the cookie is set now
        Router.push('/dashboard');
      } else {
        const errText = await res.text();
        setError(errText || "Login failed");
      }
    } catch (err) {
      console.error("Network error:", err);
      setError("Unable to login. Please try again.");
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-100">
      <form onSubmit={submit} className="bg-white p-6 rounded shadow-md w-full max-w-sm">
        <h1 className="text-2xl mb-4">Log In</h1>
        {error && <p className="text-red-600 mb-2">{error}</p>}
        <input type="email" className="w-full p-2 border mb-4" 
               placeholder="Email" value={email} onChange={e => setEmail(e.target.value)} required/>
        <input type="password" className="w-full p-2 border mb-4" 
               placeholder="Password" value={password} onChange={e => setPassword(e.target.value)} required/>
        <button type="submit" className="w-full bg-blue-600 text-white py-2 rounded">Login</button>
      </form>
    </div>
  );
}

In this snippet, we POST to our Rust API (the URL is taken from an environment variable for the frontend, e.g. NEXT_PUBLIC_API_URL might be http://localhost:3000 if the Rust server runs there). We set credentials: 'include' to allow cookie reception (for JWT cookie). If login is successful (res.ok), we redirect the user to the dashboard page. Errors are displayed above the form.

The register page would be similar, posting to /api/register. Upon successful registration, one might auto-login or redirect to login page.

Dashboard Page (Listing MCP Servers): Once logged in, the user sees a dashboard of their MCP servers. We create a protected page pages/dashboard.tsx. We can use Next.js server-side rendering to fetch the list of servers before rendering (so the data is there on initial load), or fetch client-side in a useEffect. If using SSR, we need to forward the user’s cookie to the API. This can be done by writing a Next.js getServerSideProps that reads cookies from the request and calls our API. Alternatively, a simpler approach: since we set a cookie, the browser will include it automatically when our Next.js frontend calls the API (if same domain or if CORS is setup with credentials). For local dev with separate domains, ensure CORS on Rust API allows our origin and credentials.

A simple client-side fetch approach:

// pages/dashboard.tsx
import { useEffect, useState } from 'react';

type Server = {
  id: number;
  name: string;
  server_type: string;
  status: string;
  created_at: string;
};

export default function Dashboard() {
  const [servers, setServers] = useState<Server[]>([]);
  const [error, setError] = useState("");
  
  useEffect(() => {
    fetch(`${process.env.NEXT_PUBLIC_API_URL}/api/servers`, { credentials: 'include' })
      .then(res => {
        if (!res.ok) throw new Error(`Failed to fetch servers: ${res.status}`);
        return res.json();
      })
      .then(data => setServers(data))
      .catch(err => setError(err.message));
  }, []);

  return (
    <div className="p-4">
      <h1 className="text-3xl font-bold mb-4">My MCP Servers</h1>
      {error && <p className="text-red-500">Error: {error}</p>}
      <table className="min-w-full bg-white shadow rounded">
        <thead className="bg-gray-50">
          <tr>
            <th className="text-left p-2">Name</th>
            <th className="text-left p-2">Type</th>
            <th className="text-left p-2">Status</th>
            <th className="text-left p-2">Created</th>
            <th className="p-2"></th>
          </tr>
        </thead>
        <tbody>
          {servers.map(s => (
            <tr key={s.id} className="border-b">
              <td className="p-2">{s.name}</td>
              <td className="p-2">{s.server_type}</td>
              <td className="p-2">{s.status}</td>
              <td className="p-2">{new Date(s.created_at).toLocaleString()}</td>
              <td className="p-2">
                <a href={`/servers/${s.id}`} className="text-blue-600 hover:underline">Manage</a>
              </td>
            </tr>
          ))}
          {servers.length === 0 && !error && (
            <tr><td colSpan={5} className="p-2 text-center">No servers yet.</td></tr>
          )}
        </tbody>
      </table>
      <button onClick={() => (window.location.href = '/servers/new')}
              className="mt-4 px-4 py-2 bg-green-600 text-white rounded">
        + Deploy New Server
      </button>
    </div>
  );
}

This displays the list in a table. The Manage link goes to a detail page for a specific server (pages/servers/[id].tsx in Next.js dynamic routes), and we have a button to create a new server which navigates to a creation page or could open a modal.

Creating a Server (Frontend): On the create page (pages/servers/new.tsx), we can present a form to select the server type and input config. If the platform has preset types, we might fetch the catalog of types from an API (or define them in the frontend). For simplicity, maybe we have a fixed list:

const SERVER_TYPES = ["PostgreSQL", "Slack", "WeatherAPI", "OpenAPI"];

The form might include a dropdown for type, a name field, and conditional inputs for config (for example, if “PostgreSQL” is selected, we might ask for a DB connection string; for “Slack”, ask for a Slack token, etc.). Implementing dynamic forms is beyond scope, but one can simply treat config as a JSON text for now or a couple of generic fields.

Submitting the form calls POST /api/servers. We should then redirect to the dashboard or server detail page. The new server will likely show up with status “creating” initially – we can poll or use SSE to update its status to “running” in the UI.

Server Detail & Actions: At pages/servers/[id].tsx, we show details about that server. We fetch the server info (like name, type, status, api_key maybe) and also fetch usage metrics. We provide controls: e.g. “Start” (if stopped), “Stop” (if running), “Delete”. Clicking those should call the respective API endpoints (with proper confirmation for delete). Upon clicking “Stop”, for instance:

await fetch(`${apiUrl}/api/servers/${id}/stop`, { method: 'POST', credentials: 'include' });

After which we might update state to mark status as “stopped” (or refetch server info).

We can also display the API Key and the endpoint URL that the user’s AI agent should use to connect. For example:

<div className="bg-gray-100 p-4 rounded">
  <p><strong>Deployment URL:</strong> https://<code>{server.id}.mcp.example.com</code></p>
  <p><strong>API Key:</strong> <code>{server.api_key}</code></p>
  <p className="text-sm text-gray-600">Use this URL and API key in your AI assistant configuration to connect.</p>
</div>

(If we didn’t implement subdomains, this could be a placeholder or something like http://host:port we computed. But ideally, we mimic the AnyContext style with subdomain.)

Usage Metrics UI: For historical metrics, we could show a chart. Using a library like Chart.js or Recharts can make attractive visuals. For instance, to integrate Chart.js we’d install react-chartjs-2 and chart.js and create a line chart of daily usage counts  . If we have data like an array of dates and counts, it’s straightforward to pass it to a <Line> component from react-chartjs-2.

For real-time metrics, we can use the SSE endpoint we made. In React, the simplest is to use the browser’s EventSource API:

// In server detail component
useEffect(() => {
  const evtSource = new EventSource(`${apiUrl}/api/servers/${id}/metrics/stream`, { withCredentials: true });
  evtSource.onmessage = (event) => {
    if (event.data) {
      const obj = JSON.parse(event.data);
      setLiveMetrics(obj); // update some state with the new data
    }
  };
  evtSource.onerror = (err) => {
    console.error("SSE error:", err);
    evtSource.close();
  };
  return () => evtSource.close();
}, [id]);

Now liveMetrics state will update whenever the server pushes an event (for example, number of requests per minute as we coded earlier). We can display that in the UI (e.g. “Current QPS: X” or updating a chart dynamically). This gives the user immediate feedback on usage without refreshing .

Responsive Design: Throughout the frontend, use Tailwind’s responsive utilities to ensure the site works on mobile and desktop. For example, on the dashboard table, we might hide less important columns on small screens or use a card layout instead of a wide table on a narrow viewport. Tailwind makes it easy: e.g., <td className="hidden sm:table-cell"> could hide a cell on mobile, showing it from sm breakpoint up. We also used classes like min-h-screen flex items-center justify-center for the login container – these ensure the page is centered vertically and horizontally, and will naturally adapt to various screen sizes. The design principle is mobile-first: start with a single-column or stacked layout, then add md: prefixes to create side-by-side components on larger screens  .

For instance, we might design the dashboard as a two-column layout on desktop: a sidebar with navigation and a main content area. Using Tailwind, we could do:

<div className="md:flex">
  <aside className="md:w-1/4 p-4 bg-gray-800 text-white"> ...sidebar links... </aside>
  <main className="md:w-3/4 p-6"> ...main dashboard content... </main>
</div>

On mobile, without the md: prefixes, the aside and main will stack (full width each) . On medium screens and up, aside takes 25% width and main 75%, side by side.

We can also employ ready-made UI components or examples – e.g., Tailwind UI or Flowbite components – for a polished look, but manual composition with Tailwind utilities as above works fine.

Best Practices and Considerations

Security: We’ve applied several security best practices:
	•	Password hashing with Argon2id (memory-hard, recommended algorithm) .
	•	JWT in HttpOnly cookies to prevent XSS stealing, combined with Secure and SameSite=Strict to prevent CSRF and ensure tokens only go over HTTPS .
	•	Validations and error handling to avoid undefined behavior (checking input lengths, handling DB errors).
	•	Access control at every API (auth middleware and owner checks) – never assume the client will hide unauthorized options; always enforce on server.
	•	SQL Injection is mitigated by using parameterized queries ($1, $2 placeholders with sqlx) or Diesel’s safe query builder, so user input isn’t directly interpolated into SQL.
	•	CORS: If our frontend is served on a different domain than the Rust API, configure the Axum server’s CORS to allow that origin and credentials. The tower_http::cors::CorsLayer can be used to allow methods and headers and set allow_credentials(true) so that cookies work cross-site.
	•	API Key for MCP servers: Each deployed server has an api_key and the server’s container should require this for any client communication (like AnyContext uses an x-api-key header ). This prevents others from connecting to your MCP server URL if they somehow guess it. The platform should generate secure random API keys (the UUID approach is okay, though a longer random string or using a crypto random generator is even better).
	•	Resource Limits: As mentioned, set limits on container resources to prevent abuse. Also consider implementing quotas (e.g., a user can only create N servers, or sessions, or certain rate limits) to protect against misuse.
	•	Logging and Monitoring: Use tracing to log important events (user logins, errors, container starts/stops) with appropriate levels. In production, aggregate logs and use monitoring for the infrastructure (Prometheus/Grafana for resource use, etc.). We can expose a health check endpoint (like GET /api/health) that just returns 200 OK – useful for load balancers or uptime monitors.

Scalability: Our design is mostly stateless in the web tier (thanks to JWT and database storage). The Rust server can be replicated behind a load balancer – all instances connect to the same Postgres and same Docker daemon or cluster. Postgres itself can be scaled (read replicas, etc.) if needed, but given typical usage (mostly config data and logs), a single instance or managed DB should handle quite a lot. If one host can’t handle all MCP containers, that’s where an orchestrator (Kubernetes) would schedule containers on multiple nodes. The platform could then become more complex (the control plane might need to decide which node to launch a container on). However, initially you might simply increase the server’s VM size.

To scale WebSockets or SSE (for real-time metrics), consider using a message broker or a pub-sub (like Redis pub/sub or an event bus) if you have multiple web server instances, so that all instances can broadcast events to users regardless of which instance is handling the container. Another approach is to push metrics to a central time-series DB and have the frontend poll that – simpler but less instantaneous.

Maintainability: We modularized code by splitting responsibilities:
	•	Route handlers for each resource (users, servers, sessions, metrics) can live in separate modules in Rust.
	•	A separate module or service object for interacting with Docker (to keep that logic isolated).
	•	On the frontend, use Next.js pages for routing but factor out components for reuse (e.g., a ServerList component, a ServerForm, a MetricChart component, etc.). This keeps the code DRY and easier to test or update.
	•	Writing unit tests for Rust handlers (using something like axum::body::Body to simulate requests) can ensure our auth logic and DB interactions work as expected. Integration testing with a temporary database and maybe a dummy Docker client (for not actually spawning containers in tests) would be valuable.

Modern UI/UX: Finally, ensure the UI is clean and intuitive. Using Tailwind, we can quickly implement modern design trends:
	•	Dark mode support: as seen in the Tailwind classes above, e.g., dark:bg-gray-800 – Tailwind can automatically support a dark theme if we add media or class strategy in config. This could be a nice touch.
	•	Interactive feedback: Show loading spinners when actions are in progress, confirm modal on deletions, toast notifications on success/failure. The Next.js app could use a library like react-hot-toast for notifications.
	•	Responsive nav: Perhaps a hamburger menu on mobile to show the sidebar links.
	•	Consistent styling: define a few reusable style classes or use Tailwind’s theming to keep colors consistent (e.g., define primary color, etc.).
	•	Accessibility: Use proper HTML elements (forms, labels, buttons) and Tailwind’s accessibility utilities if needed (like sr-only for screen reader text).

By following this guide, you can assemble a full-stack system that mimics AnyContext: a Rust backend managing user accounts and containerized context servers, and a Next.js frontend for a seamless user experience. The result is a scalable “Context-as-a-Service” platform – enabling users to spin up connectors that bridge AI and external data with ease and security.

References:
	•	AnyContext architecture and MCP concept  
	•	Axum and Diesel for high-performance Rust APIs  
	•	Password hashing best practices (Argon2id) 
	•	JWT storage security (HttpOnly cookies vs localStorage)  
	•	Docker container management with Bollard (Rust)  
	•	Server-Sent Events for real-time updates  
	•	Tailwind CSS responsive design patterns  
