use std::{
    net::{SocketAddr, TcpListener},
    sync::Arc,
    time::Duration,
};

use actix_server::{Server, ServerHandle};
use actix_web::{
    http,
    web::{method, Data},
    HttpResponse, HttpServer,
};
use sqlx::{
    postgres::{PgConnectOptions, PgPoolOptions},
    PgPool,
};
use tokio::runtime::Runtime;

pub struct Application {
    pub port: u16,
    server: Option<Server>,
    server_handle: Arc<ServerHandle>,
}

impl Application {
    pub fn new(port: u16, server: Server) -> Self {
        Self {
            port,
            server_handle: Arc::new(server.handle()),
            server: Some(server),
        }
    }

    pub async fn run_until_stopped(mut self) -> Result<(), std::io::Error> {
        if let Some(server) = self.server.take() {
            println!("run until stop");
            server.await?;
        }

        println!("Stopped!");

        Ok(())
    }
}

impl Drop for Application {
    fn drop(&mut self) {
        println!("Drop.");
        let handle = self.server_handle.clone();

        Runtime::new().unwrap().block_on(async move {
            // Never printed.
            handle.stop(true).await;
            println!("Cleaned up");
        })
    }
}

async fn handler(pool: Data<PgPool>) -> HttpResponse {
    let mut queries = vec![];
    for _n in 0..10 {
        let foo = sqlx::query!("select 1 as whatever").fetch_one(pool.as_ref());

        queries.push(foo);
    }

    // Run these all consecutively 
    let queries: Vec<_> = queries.into_iter().map(Box::pin).collect();
    futures::future::select_all(queries).await.0.unwrap();

    HttpResponse::Ok().body(format!("{}", 10))
}

pub async fn get_test_pool(pool_opts: PgPoolOptions, conn_opts: PgConnectOptions) -> PgPool {
    let pool = pool_opts
        .max_connections(10)
        .after_release(|_conn, _| Box::pin(async move { 
            println!("TEST CONN RELEASED");
            Ok(false) 
        }))
        .idle_timeout(Some(Duration::from_secs(600)))
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(conn_opts)
        .await
        .unwrap();

    println!("Pool: {:?}", pool);

    pool
}

fn build_app(pool: &PgPool) -> Application {
    let pool = pool.clone();
    let server = HttpServer::new(move || {
        actix_web::App::new()
            .app_data(Data::new(pool.clone()))
            .route("/test", method(http::Method::GET).to(handler))
    });

    let listener = TcpListener::bind(Into::<SocketAddr>::into(([0, 0, 0, 0], 0))).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = server.listen(listener).unwrap();

    Application::new(port, server.run())
}

seq_macro::seq!(N in 0..10 {
    #[sqlx::test]
    pub async fn test_~N(
        pool_opts: PgPoolOptions,
        conn_opts: PgConnectOptions,
    ) -> sqlx::Result<()> {
        let pool = get_test_pool(pool_opts, conn_opts).await;
        let app = build_app(&pool);
        let port = app.port;
        let _join_handle = tokio::spawn(app.run_until_stopped());

        let client = reqwest::Client::new();

        let resp = client
            .get(&format!("http://0.0.0.0:{}/test", port))
            .send()
            .await
            .unwrap();

        let content = resp.text().await.unwrap();
        // just force it to fail
        assert_eq!(content, "-1".to_owned());

        Ok(())
    }
});