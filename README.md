# Claudi CLI Workspace

Welcome to the **Claudi CLI** workspace. This monorepo brings together two powerful CLI agent implementations, providing a centralized environment for exploring and developing terminal-based AI agents.

**The primary goal of this codebase is to provide a path for integrating Gemini CLI's robust authentication mechanisms (such as "Sign in with Google") into the high-performance `claw-code` Rust client.**

## 📂 Repository Components

This workspace consists of two primary projects:

### 1. [Claw Code](./claw-code)
**Claw Code** is a high-performance Rust implementation of the `claw` CLI agent harness. It focuses on being a build-from-source agent that maintains parity with established agent workflows.

*   **Language:** Rust
*   **Key Features:** Deterministic mock-service harness, parity-driven development, and lightweight execution.
*   **Quick Start:**
    ```bash
    cd claw-code/rust
    cargo build --workspace
    ./target/debug/claw doctor
    ```

### 2. [Gemini CLI](./gemini-cli)
**Gemini CLI** is an extensible, terminal-first AI agent powered by Google's Gemini models. It provides a rich feature set including Google Search grounding, file operations, and MCP (Model Context Protocol) support.

*   **Language:** TypeScript (Node.js/React/Ink)
*   **Key Features:** 1M+ token context window, built-in tool suite, and deep GitHub integration.
*   **Quick Start:**
    ```bash
    cd gemini-cli
    npm install
    npm run build
    npm run start
    ```

---

## 🚀 Why this Workspace?

This combined repository allows for:
- **Cross-model experimentation:** Compare Gemini-based agents with Rust-based implementations.
- **Unified Development:** Shared environment for developing and testing multiple agent architectures.
- **Architectural Reference:** Explore different approaches to CLI agent design, from TypeScript/React to Rust.

## 📖 Further Documentation

For detailed information on each project, please refer to their respective documentation:

- **Claw Code Docs:** [claw-code/README.md](./claw-code/README.md)
- **Gemini CLI Docs:** [gemini-cli/README.md](./gemini-cli/README.md)

---

Built with ❤️ by the open source community.
