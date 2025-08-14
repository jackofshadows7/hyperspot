use modkit_macros::module;

#[module(name="x", caps=[stateful], lifecycle(entry="serve"), lifecycle(entry="serve"))]
pub struct X;

fn main() {}
