// substantial portion from `cargo-cacher:/src/main.rs`
// https://github.com/ChrisMacNaughton/cargo-cacher

use std::path::Path;

use iron::prelude::*;
use iron::status;
use router::Router;

use crate::{
    middleware::cors::CorsMiddleware,
    mirror::{MirrorError, ServeSection},
};

#[derive(Clone, Debug)]
pub struct CargoRequest {
    /// crate name, ex: cargo-cacher
    name: String,
    /// major.minor.patch
    version: String,
    /// Cache hit?
    hit: bool,
    /// Filesize in bytes
    size: i64,
}

pub fn serve(path: &Path) -> Result<(), MirrorError> {
    let serve: ServeSection;
    match crate::mirror::load_mirror_toml(path)?.serve {
        Some(serve_section) => {
            serve = serve_section;
        }
        _ => panic!("serve section of the Mirror.toml is missing."),
    }

    // own path to use in request processing
    let path = path.clone().to_owned();
    // let path2 = &path;

    // web server to handle DL requests
    let host = format!(":::{}", serve.port);
    let router = router!(
        // old crates.io API?
        download: get "api/v1/crates/:crate_name/:crate_version/download" => {
            let path = path.clone();
            move |request: &mut Request|
                crates_download(request, &path)
        },
        // this one works
        download2: get "crates/:crate_name/:crate_version/download" => {
            let path = path.clone();
            move |request: &mut Request|
                crates_download(request, &path)
        },
        rustup_dist: get "dist/**" => {
            let path = path.clone();
            move |request: &mut Request|
                simple_download(request, &path)
        },
        rustup_update: get "rustup/**" => {
            let path = path.clone();
            move |request: &mut Request|
                simple_download(request, &path)
        },
        head: get "index/*" => {
            let path = path.clone();
            move |request: &mut Request|
                crate::git::git(request, &path)
        },
        index: get "index/**/*" => {
            let path = path.clone();
            move |request: &mut Request|
                crate::git::git(request, &path)
        },
        head: post "index/*" => {
            let path = path.clone();
            move |request: &mut Request|
                crate::git::git(request, &path)
        },
        index: post "index/**/*" => {
            let path = path.clone();
            move |request: &mut Request|
                crate::git::git(request, &path)
        },
        root: any "/" => log,
        query: any "/*" => log,
    );
    let mut chain = Chain::new(router);
    chain.link_after(CorsMiddleware);
    println!("Listening on {}", host);
    // Iron::new(chain).http(host).unwrap();
    Iron::new(chain).http(&host[..]).unwrap();

    Ok(())
}

pub fn log(req: &mut Request) -> IronResult<Response> {
    eprintln!("Whoops! {:?}", req);
    Ok(Response::with((status::Ok, "Ok")))
}

fn crates_download(req: &mut Request, path: &Path) -> IronResult<Response> {
    let ref crate_name = req
        .extensions
        .get::<Router>()
        .unwrap()
        .find("crate_name")
        .unwrap();
    let ref crate_version = req
        .extensions
        .get::<Router>()
        .unwrap()
        .find("crate_version")
        .unwrap();
    eprintln!("Downloading: {}:{}", crate_name, crate_version);
    eprintln!("Raw request: {:?}", req);
    let crate_path = path
        .join("crates")
        .join(crate_name)
        .join(crate_version)
        .join("download");

    if crate_path.exists() {
        // eprintln!("path {:?} exists!", crate_path);
        Ok(Response::with((status::Ok, crate_path)))
    } else {
        eprintln!("Could not find crate in path: {:?}", crate_path);
        Ok(Response::with((
            status::NotFound,
            format!("Could not find crate ({}) in offline mirror.", crate_name),
        )))
    }
}

fn simple_download(req: &mut Request, path: &Path) -> IronResult<Response> {
    // let directory = req.url.path().first().unwrap();
    // println!("req dir  => {}", directory);
    eprintln!("Raw request: {:?}", req);
    println!("req path => {:?}", req.url.path());

    let file_path = path.join(req.url.path().join("/"));

    eprintln!("Downloading: {:?}", file_path);

    if file_path.exists() {
        // eprintln!("path {:?} exists!", crate_path);
        Ok(Response::with((status::Ok, file_path)))
    } else {
        eprintln!("Could not find file in path: {:?}", file_path);
        Ok(Response::with((
            status::NotFound,
            format!(
                "Could not find file ({}) in offline mirror.",
                req.url.path().join("/")
            ),
        )))
    }
    //  else {
    //     debug!("path {:?} doesn't exist!", path);

    //     match fetch(
    //         &path,
    //         &config.upstream,
    //         &config.index_path,
    //         &crate_name,
    //         &crate_version,
    //     ) {
    //         Ok(_) => {
    //             let _ = stats.send(CargoRequest {
    //                 name: crate_name.to_string(),
    //                 version: crate_version.to_string(),
    //                 hit: false,
    //                 size: size(&path) as i64,
    //             });
    //             Ok(Response::with((status::Ok, path)))
    //         }
    //         Err(e) => {
    //             error!("{:?}", e);
    //             return Ok(Response::with((
    //                 status::ServiceUnavailable,
    //                 "Couldn't fetch from Crates.io",
    //             )));
    //         }
    //     }
    // }

    // Ok(Response::with((status::Ok, "Ok")))
}
