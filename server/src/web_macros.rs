macro_rules! path {
  (let /{$seg:pat} $(/$tail:tt)+ = $url:expr) => {
      let url = $url;
      let url = url.strip_prefix("/").ok_or($crate::error::Error::NotFound("Route"))?;
      let split_index = url.find('/').ok_or($crate::error::Error::NotFound("Route"))?;
      let $seg = &url[..split_index];
      let url = &url[split_index..];
      path!(let $(/$tail)+ = url);
  };
  (let /{$seg:ident: $sty:ty} $(/$tail:tt)+ = $url:expr) => {
      let url = $url;
      let url = url.strip_prefix("/").ok_or($crate::error::Error::NotFound("Route"))?;
      let split_index = url.find('/').ok_or($crate::error::Error::NotFound("Route"))?;
      let $seg = url[..split_index].parse::<$sty>().map_err(|_| $crate::error::Error::NotFound("Route"))?;
      let url = &url[split_index..];
      path!(let $(/$tail)+ = url);
  };
  (let /$seg:ident $(/$tail:tt)+ = $url:expr) => {
      let url = $url;
      let url = url.strip_prefix(concat!("/", stringify!($seg))).ok_or($crate::error::Error::NotFound("Route"))?;
      path!(let $(/$tail)+ = url);
  };
  (let / = $url:expr) => {
      let url = $url;
      if url != "/" {
          return Err($crate::error::Error::NotFound("Route"))
      }
  };
  (let /$seg:ident = $url:expr) => {
      let url = $url;
      if url != concat!("/", stringify!($seg)) {
          return Err($crate::error::Error::NotFound("Route"))
      }
  };
  (let /{$seg:ident} = $url:expr) => {
      let url = $url.strip_prefix("/").ok_or($crate::error::Error::NotFound("Route"))?;
      let $seg = url;
  };
  (let /{$seg:ident: $sty:ty} = $url:expr) => {
      let url = $url.strip_prefix("/").ok_or($crate::error::Error::NotFound("Route"))?;
      let $seg = url.parse::<$sty>().map_err(|_| $crate::error::Error::NotFound("Route"))?;
  };
}

/// Create a http handler for a specific route.
/// ```
/// route! {
///     (GET /foo/{bar: usize}/{baz})
///     my_route(request: &Request, arg: &Arg) {
///         Ok(Response::html(format!("hello {} world", bar)))
///     }
/// };
/// ```
///
/// Will make a route function called `my_route` accepting the args `request: &Request, arg: &Arg`
/// if the route matches, the second path segment will be parsed as a `usize` and extracted to the
/// variable `bar` and the third and any following segments will be captured in the variable `baz`
/// If the route doesn't match, the functions will return a 404
macro_rules! route {
  {( $method:ident $($path:tt)*) $id:ident($request:ident: &Request $(, $arg:ident: $arg_ty:ty)*) $b:block} => {
      fn $id ($request: &Request $(, $arg: $arg_ty)*) -> Response {
          let f = move || -> Result<Response, $crate::error::Error> {
              if $request.method() != stringify!($method) {
                  return Err($crate::error::Error::NotFound("Method"))
              }
              path!(let $($path)* = $request.url());
              $b
          };
          f().unwrap_or_else(Into::into)
      }
  };
}

#[cfg(test)]
macro_rules! assert_extract {
    (path($($args:tt)*); $($is:ident == $should:literal);*) => {
        {
            let f = || -> Result<rouille::Response, $crate::error::Error>  {
                path!($($args)*);
                $(
                    assert_eq!($is, $should);
                )*
                Ok(rouille::Response::empty_204())
            };
            f().unwrap()
        }
    };
}

#[cfg(test)]
macro_rules! assert_bail {
    (path($($args:tt)*)) => {
        {
            let f = || -> Result<rouille::Response, $crate::error::Error>  {
                path!($($args)*);
                Ok(rouille::Response::empty_204())
            };
            f().unwrap_err()
        }
    };
}

#[test]
fn test_paths() {
    let path = "/foo";

    assert_extract!(path(let /foo = path););
    assert_extract!(path(let /{bar} = path); bar == "foo"); // an &str capture
    assert_extract!(path(let /{bar: String} = path); bar == "foo"); // a `FromStr` capture
    assert_bail!(path(let /bar = path)); // a non match causes a bail
    assert_bail!(path(let /{_bar: u32} = path)); // a parsing error from `FromStr` causes a bail
    assert_bail!(path(let / = path));
    assert_bail!(path(let /foo/bar = path)); // unmatched segments cause a bail
    assert_bail!(path(let /foo/{_bar} = path));
    assert_bail!(path(let /foo/{_bar: String} = path));
    assert_bail!(path(let /{_foo}/bar = path));
    assert_bail!(path(let /{_foo: String}/bar = path));
    assert_bail!(path(let /bar/foo = path));

    let path = "/foo/bar";

    assert_bail!(path(let /foo = path));
    assert_extract!(path(let /{baz} = path); baz == "foo/bar");
    assert_extract!(path(let /{bar: String} = path); bar == "foo/bar"); // the last capture captures the tail
    assert_bail!(path(let /bar = path));
    assert_bail!(path(let /{_bar: u32} = path));
    assert_bail!(path(let / = path));
    assert_extract!(path(let /foo/bar = path););
    assert_extract!(path(let /foo/{bar} = path); bar == "bar");
    assert_extract!(path(let /foo/{bar: String} = path); bar == "bar");
    assert_extract!(path(let /{foo}/bar = path); foo == "foo");
    assert_extract!(path(let /{foo: String}/bar = path); foo == "foo");
    assert_bail!(path(let /bar/foo = path));

    let path = "/10/-4";

    assert_extract!(path(let /{a: u8}/{b: i8} = path); a == 10; b == -4);

    let path = "/";

    assert_extract!(path(let / = path););
}
