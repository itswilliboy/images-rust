#![feature(proc_macro_hygiene, decl_macro)]
#[macro_use]
extern crate rocket;

use nanoid::nanoid;
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::Header;
use rocket::http::{ContentType, Status};
use rocket::request::FromRequest;
use rocket::request::Outcome;
use rocket::response::Responder;
use rocket::{Request, Response};
use rocket_db_pools::sqlx::{self, Row};
use rocket_db_pools::{Connection, Database};
use std::fs::{read, remove_file};
use std::io::Cursor;

struct BufferResponse(Vec<u8>, String);

impl<'r> Responder<'r, 'static> for BufferResponse {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        let response = Response::build()
            .header(Header::new("Content-Type", self.1))
            .streamed_body(Cursor::new(self.0))
            .status(Status::Ok)
            .finalize();

        Ok(response)
    }
}
struct Authorization(String);

#[derive(Debug)]
enum AuthorizationError {
    Missing,
    Invalid,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Authorization {
    type Error = AuthorizationError;

    async fn from_request(
        req: &'r rocket::Request<'_>,
    ) -> rocket::request::Outcome<Self, Self::Error> {
        match req.headers().get_one("Authorization") {
            Some(key) if key == "beans" => Outcome::Success(Authorization(key.to_owned())),
            Some(_) => Outcome::Failure((Status::Unauthorized, AuthorizationError::Invalid)),
            None => Outcome::Failure((Status::Unauthorized, AuthorizationError::Missing)),
        }
    }
}

#[derive(Database)]
#[database("database")]
struct DB(sqlx::PgPool);

#[get("/")]
fn index() -> &'static str {
    "Hello, World!"
}

#[get("/favicon.ico")]
async fn favicon() -> NamedFile {
    NamedFile::open("favicon.ico").await.ok().unwrap()
}

#[get("/<filename>")]
async fn get(mut db: Connection<DB>, filename: &str) -> Result<BufferResponse, Status> {
    let split: Vec<&str> = filename.split(".").collect();
    let name = split[0];
    let resp = sqlx::query("SELECT * FROM images WHERE id = $1")
        .bind(name)
        .fetch_optional(&mut *db)
        .await;

    match resp {
        Ok(Some(row)) => {
            let buffer: Vec<u8> = row.try_get("image_data").unwrap();
            let content_type: &str = row.try_get("mimetype").unwrap_or("image/png");

            Ok(BufferResponse(buffer, content_type.into()))
        }
        _ => Err(Status::NotFound),
    }
}

fn get_id() -> String {
    let chars: [char; 52] = [
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J',
        'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    ];
    nanoid!(10, &chars)
}

#[post("/upload", data = "<file>", format = "multipart/form-data")]
async fn upload(
    mut db: Connection<DB>,
    _auth: Authorization,
    mut file: Form<TempFile<'_>>,
) -> Result<(Status, (ContentType, String)), Status> {
    let ext = {
        let mimetype = file.content_type().unwrap();
        mimetype.extension().unwrap().to_string()
    };

    let id = get_id();
    let path = format!("temp/{}.{}", id, ext);

    file.persist_to(&path)
        .await
        .expect("Something went wrong when creating tempfile.");
    let image_data = read(&path).unwrap();
    let mimetype = file.content_type().unwrap().to_string();

    let resp = sqlx::query("INSERT INTO images (id, image_data, mimetype) VALUES ($1, $2, $3)")
        .bind(id.clone())
        .bind(image_data)
        .bind(mimetype.to_string())
        .execute(&mut *db)
        .await;

    match resp {
        Ok(_) => {
            match remove_file(path) {
                _ => (),
            };
            Ok((
                Status::Ok,
                (
                    ContentType::JSON,
                    format!(
                        "{{\"id\": \"{}\", \"url\": \"https://i.itswilliboy.com/{}.{}\"}}",
                        id, id, ext
                    )
                    .to_owned(),
                ),
            ))
        }
        Err(e) => {
            eprintln!("{:?}", e);
            Err(Status::InternalServerError)
        }
    }
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .attach(DB::init())
        .mount("/", routes![index, favicon, get, upload])
}
