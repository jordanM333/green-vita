//! Embedded CI identity so device screenshots identify the installed binary.
pub(crate) const NUMBER: &str = match option_env!("GREENVITA_BUILD_NUMBER") {
    Some(number) => number,
    None => "local",
};

pub(crate) const REVISION: &str = match option_env!("GREENVITA_BUILD_SHA") {
    Some(revision) => revision,
    None => "local",
};
