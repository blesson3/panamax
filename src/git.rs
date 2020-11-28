// substantial portion from `cargo-cacher:/src/git.rs`
// https://github.com/ChrisMacNaughton/cargo-cacher

// use std::ascii::AsciiExt;
use std::io;
use std::io::prelude::*;
use std::process::{Command, Stdio};
use std::{collections::HashMap, path::Path};

// Iron Stuff
use iron::headers::ContentType;
use iron::mime::{Mime, SubLevel, TopLevel};
use iron::prelude::*;
use iron::response::BodyReader;
use iron::status::{self, Status};

pub fn git(req: &mut Request, path: &Path) -> IronResult<Response> {
    let repo_path = path.join("crates.io-index");

    eprintln!("path: {:?}", &repo_path);
    eprintln!("Raw GIT request: {:?}", req);
    // let content_type: &str = match req.headers.get::<ContentType>() {
    //     Some(s) => {
    //         s.get_param("Content-Type")
    //             .map(|s| s.as_str())
    //             .unwrap_or("")
    //     }
    //     None => "",
    // };

    let content_type: String = match req.headers.get() {
        Some(&ContentType(Mime(TopLevel::Application, SubLevel::Ext(ref s), _))) => {
            format!("application/{}", s)
        }
        _ => "".into(),
    };

    let path_info = if req.url.path().join("/").starts_with("/") {
        req.url.path().join("/").to_string()
    } else {
        format!("/{}", req.url.path().join("/"))
    };
    let path_info = path_info.replace("/index/", "/");
    eprintln!("path_info: {}", path_info);
    let method = format!("{:?}", req.method).to_ascii_uppercase();
    let query_string = req.url.query().unwrap_or("");
    let remote_addr = req.remote_addr.to_string();
    let mut cmd = Command::new("git");
    cmd.arg("http-backend");
    // Required environment variables
    cmd.env("REQUEST_METHOD", method);
    cmd.env("GIT_PROJECT_ROOT", &repo_path);
    cmd.env("PATH_INFO", path_info);

    cmd.env("REMOTE_USER", "");
    cmd.env("REMOTE_ADDR", remote_addr);
    cmd.env("QUERY_STRING", query_string);
    cmd.env("CONTENT_TYPE", content_type);
    cmd.stderr(Stdio::inherit())
        .stdout(Stdio::piped())
        .stdin(Stdio::piped());
    let mut p = match cmd.spawn() {
        Ok(s) => s,
        Err(_) => {
            return Ok(Response::with((
                status::InternalServerError,
                "Failed to run git",
            )))
        }
    };
    let _ = io::copy(&mut req.body, &mut p.stdin.take().unwrap());
    // Parse the headers coming out, and the pass through the rest of the
    // process back down the stack.
    //
    // Note that we have to be careful to not drop the process which will wait
    // for the process to exit (and we haven't read stdout)

    let out = match p.wait_with_output() {
        Ok(s) => s,
        _ => {
            return Ok(Response::with((
                status::InternalServerError,
                "Failed to run git",
            )))
        }
    };
    // debug!("Out: {}", String::from_utf8_lossy(out));
    let mut rdr = io::BufReader::new(io::Cursor::new(out.stdout));

    let mut headers = HashMap::new();
    for line in rdr.by_ref().lines() {
        let line = match line {
            Ok(s) => s,
            _ => break,
        };
        if line == "" || line == "\r" {
            break;
        }

        let mut parts = line.splitn(2, ':');
        let key = parts.next().unwrap();
        let value = parts.next().unwrap();
        let value = &value[1..];
        headers
            .entry(key.to_string())
            .or_insert(Vec::new())
            .push(value.to_string());
    }

    // let mut buf = Vec::new();
    // let _ = rdr.read_to_end(&mut buf);

    let content_type = headers
        .get("Content-Type")
        .unwrap_or(&vec![])
        .first()
        .unwrap_or(&"".to_string())
        .clone()
        .split("/")
        .last()
        .unwrap_or("")
        .into();
    Ok(Response::with((
        Status::Ok,
        BodyReader(rdr),
        Mime(TopLevel::Application, SubLevel::Ext(content_type), vec![]),
    )))
}
