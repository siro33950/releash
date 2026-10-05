use prost_reflect::{DescriptorPool, DynamicMessage, Value};
use std::sync::LazyLock;

static POOL: LazyLock<DescriptorPool> = LazyLock::new(|| {
    DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .expect("client descriptors")
});

pub(crate) fn pool() -> &'static DescriptorPool {
    &POOL
}

pub(crate) fn option(options: &DynamicMessage, name: &str) -> Value {
    let extension = POOL
        .get_extension_by_name(&format!("releash.client.v1.{name}"))
        .expect("protocol option");
    options.get_extension(&extension).into_owned()
}

pub(crate) fn client_service() -> prost_reflect::ServiceDescriptor {
    POOL.get_service_by_name("releash.client.v1.ClientService")
        .expect("ClientService descriptor")
}

pub(crate) fn protocol() -> u32 {
    client_service()
        .parent_file()
        .package_name()
        .rsplit('.')
        .next()
        .and_then(|version| version.strip_prefix('v'))
        .and_then(|version| version.parse().ok())
        .expect("versioned protocol package")
}
