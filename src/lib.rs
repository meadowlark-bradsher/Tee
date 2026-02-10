pub mod config;
pub mod domain;
pub mod proto_convert;
pub mod schema;
pub mod service;
pub mod store;

pub mod proto {
    tonic::include_proto!("tee");
}
