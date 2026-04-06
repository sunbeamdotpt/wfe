//! Containerd container executor for WFE.
//!
//! Runs workflow steps as isolated OCI containers via the containerd gRPC API.
//!
//! # Remote daemon support
//!
//! The executor creates named pipes (FIFOs) on the **local** filesystem for
//! stdout/stderr capture, then passes those paths to the containerd task spec.
//! The containerd shim opens the FIFOs from **its** side. This means the FIFO
//! paths must be accessible to both the executor process and the containerd
//! daemon.
//!
//! When containerd runs on a different machine (e.g. a Lima VM), you need:
//!
//! 1. **Shared filesystem** — mount a host directory into the VM so both sides
//!    see the same FIFO files. With Lima + virtiofs:
//!    ```yaml
//!    # lima config
//!    mounts:
//!      - location: /tmp/wfe-io
//!        mountPoint: /tmp/wfe-io
//!        writable: true
//!    ```
//!
//! 2. **`WFE_IO_DIR` env var** — point the executor at the shared directory:
//!    ```sh
//!    export WFE_IO_DIR=/tmp/wfe-io
//!    ```
//!    Without this, FIFOs are created under `std::env::temp_dir()` which is
//!    only visible to the host.
//!
//! 3. **gRPC transport** — Lima's Unix socket forwarding is unreliable for
//!    HTTP/2 (gRPC). Use a TCP socat proxy inside the VM instead:
//!    ```sh
//!    # Inside the VM:
//!    socat TCP4-LISTEN:2500,fork,reuseaddr UNIX-CONNECT:/run/containerd/containerd.sock &
//!    ```
//!    Then connect via `WFE_CONTAINERD_ADDR=http://127.0.0.1:2500` (Lima
//!    auto-forwards guest TCP ports).
//!
//! 4. **FIFO permissions** — the FIFOs are created with mode `0666` and a
//!    temporarily cleared umask so the remote shim (running as root) can open
//!    them through the shared mount.
//!
//! See `test/lima/wfe-test.yaml` for a complete VM configuration that sets all
//! of this up.

pub mod config;
pub mod service_provider;
pub mod step;

pub use config::{ContainerdConfig, RegistryAuth, TlsConfig, VolumeMountConfig};
pub use service_provider::ContainerdServiceProvider;
pub use step::ContainerdStep;
