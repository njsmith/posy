/// Work around an annoyance in Rust's standard traits -- if you define
/// TryFrom<&str>, then you probably also want TryFrom<String> and FromStr,
/// and the implementation is trivial in terms of TryFrom<&str>. So this macro
/// just generates the boilerplate for you.
#[macro_export]
macro_rules! try_from_str_boilerplate {
    ($name:ident) => {
        impl std::convert::TryFrom<String> for $name {
            type Error = eyre::Report;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                (&*s).try_into()
            }
        }

        impl std::str::FromStr for $name {
            type Err = eyre::Report;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.try_into()
            }
        }
    };
}

pub fn retry_interrupted<F, T>(mut f: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    loop {
        let res = f();
        match &res {
            Ok(_) => return res,
            Err(err) => {
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return res;
                }
            }
        }
    }
}
