/// One action a user holds, checked as `resource.name`. The module is the
/// part of the product it belongs to, which is how a frontend groups them.
#[derive(Debug, Clone)]
pub struct Permission {
    pub module: String,
    pub resource: String,
    pub name: String,
}
