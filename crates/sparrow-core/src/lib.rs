pub mod sparrow_engine;
pub mod sparrow_gateway;
#[cfg(feature = "compiler")]
pub mod sparrowc;
pub mod protocol;
pub mod utils;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
