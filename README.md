
# 💻 GC Stats API

The official API for **Game Changers Stats**, build in Rust for the best efficency

---

| Build Status |                       Latest Version                                                    |
|:---:|:---------------------------------------------------------------------------------------:|
| [![CI/CD Pipeline](https://github.com/GC-Stats/API/actions/workflows/main.yml/badge.svg)](https://github.com/GC-Stats/API/actions/workflows/main.yml) |![GitHub release (latest by date)](https://img.shields.io/github/v/release/GC-Stats/API) 

---

## 📋 Presentation
This repository contains the Rust API use for our API.

## 🤝 License
License: This project is licensed under a modified MIT License - see the [LICENSE](https://github.com/GC-Stats/API/blob/main/LICENSE) file for details.

## 🛠 Tech Stack

- **Webserver:** Axum
- **API Doc:** Utoipa
- **Database:** MySQL 8.0+ / MariaDB 10.11+ (With SQlx)
- **Cache & Queue:** Redis

## ⚙️ Installation

### Option 1: Docker - Recommended
The easiest way to get started without installing Rust or MySQL locally.

1. **Clone the repo:**
   ```bash
   git clone https://github.com/GC-Stats/API.git
   cd API
   ```
2. **Copy .env**
   ```bash
   cp .env.example .env
   ```
   Edit the files, and set your own variables

3. **Launch it via Docker**
   ```bash
   docker compose up -d gc_production_api
   ```

> [!WARNING]
> The database has to be made with Laravel migration, via the [Website](https://github.com/GC-Stats/Website)


### Option 2: Manual Installation (From Source)
1. **Requirements:** Rust, Cargo, MySQL & Redis
2. **Commands:**
   ```bash
   cargo run
   ```

---
## 🤝 Contributing
Interested in helping? Please refer to our [CONTRIBUTING.md](https://github.com/GC-Stats/API/blob/main/CONTRIBUTING.md) for guidelines on how to submit pull requests.
