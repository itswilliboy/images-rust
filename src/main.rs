#[macro_use]
extern crate rocket;

use nanoid::nanoid;
use rocket::form::Form;
use rocket::fs::{NamedFile, TempFile};
use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome};
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

struct Authorization(());

#[derive(Debug)]
enum AuthorisationError {
    Missing,
    Invalid,
}

static AUTH_KEY: &str = env!("AUTH_KEY");

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Authorization {
    type Error = AuthorisationError;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.headers().get_one("Authorization") {
            Some(key) if key == AUTH_KEY => Outcome::Success(Authorization(())),
            Some(_) => Outcome::Failure((Status::Unauthorized, AuthorisationError::Invalid)),
            None => Outcome::Failure((Status::Unauthorized, AuthorisationError::Missing)),
        }
    }
}

#[derive(Database)]
#[database("database")]
struct DB(sqlx::PgPool);

#[get("/")]
async fn index() -> NamedFile {
    NamedFile::open("index.html").await.ok().unwrap()
}

#[get("/favicon.ico")]
async fn favicon() -> NamedFile {
    NamedFile::open("favicon.ico").await.ok().unwrap()
}

#[get("/assets/external.svg")]
async fn external() -> NamedFile {
    NamedFile::open("./assets/external.svg").await.ok().unwrap()
}

#[get("/<filename>")]
async fn get(mut db: Connection<DB>, filename: &str) -> Result<BufferResponse, Status> {
    let split: Vec<&str> = filename.split('.').collect();
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
    const CHARS: [char; 52] = [
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J',
        'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    ];
    nanoid!(10, &CHARS)
}

#[post("/upload", data = "<file>", format = "multipart/form-data")]
async fn upload(
    mut db: Connection<DB>,
    _auth: Authorization,
    mut file: Form<TempFile<'_>>,
) -> Result<(Status, (ContentType, String)), Status> {
    let mimetype = file.content_type().ok_or(Status::BadRequest)?;
    let ext = mimetype.extension().ok_or(Status::BadRequest)?.to_string();

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
            let _ = remove_file(path);
            Ok((
                Status::Ok,
                (
                    ContentType::JSON,
                    format!(
                        r#"{{"id": "{}", "url": "http://localhost:8000/{}.{}"}}"#,
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
        .mount("/", routes![index, favicon, external, get, upload])
}
