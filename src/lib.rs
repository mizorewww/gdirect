//! gdirect's engine, shared by the `gdirect` command and the desktop client.
pub mod api;
pub mod auth;
pub mod download;
pub mod drive;
pub mod mount;
pub mod proxy;
pub mod ranges;
pub mod route;
pub mod scan;
pub mod store;
#[cfg(test)]
mod test_network;
pub mod tui;
pub mod upload;
