use std::io;

use rouille::Response;

pub enum Error {
    NotFound(&'static str),
    InternalServer,
}

impl From<Error> for Response {
    fn from(e: Error) -> Self {
        if let Error::NotFound(name) = e {
            Response::text(format!("{} not found", name)).with_status_code(404)
        } else {
            Response::text("Internal server error").with_status_code(500)
        }
    }
}

pub trait CouldFind {
    type Success;
    fn could_find(self, name: &'static str) -> Result<Self::Success, Error>;
}

impl<T> CouldFind for Result<T, io::Error> {
    type Success = T;

    fn could_find(self, name: &'static str) -> Result<Self::Success, Error> {
        self.map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                Error::NotFound(name)
            } else {
                eprintln!("Internal server error : {}\n{:?}", err, err);
                Error::InternalServer
            }
        })
    }
}
