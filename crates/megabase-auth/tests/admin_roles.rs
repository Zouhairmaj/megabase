//! Live bootstrap of the Supabase admin roles.
//!
//! CI does not start PostgreSQL (`ci.yml` has no database service). Set
//! `MEGABASE_ROLES_TEST_URL` to a superuser URL on a disposable cluster to
//! run this. The test creates and drops roles and one database on that
//! cluster.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tokio_postgres::NoTls;

#[tokio::test]
async fn fresh_cluster_admin_roles_and_degraded_attributes() {
    let Ok(url) = std::env::var("MEGABASE_ROLES_TEST_URL") else {
        eprintln!("MEGABASE_ROLES_TEST_URL unset; skipping live admin-role bootstrap");
        return;
    };

    let warnings = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = Arc::clone(&warnings);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || VecWriter(Arc::clone(&writer)))
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let (admin, connection) = tokio_postgres::connect(&url, NoTls)
        .await
        .expect("superuser connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });

    cleanup(&admin).await;
    // Generated at runtime so the tree has no hard-coded credential.
    // 32 hex digits: no quote or URL encoding. PostgreSQL quotes it with %L.
    let password = uuid::Uuid::new_v4().simple().to_string();
    let create_role: String = admin
        .query_one(
            "SELECT format(\
                 'CREATE ROLE mb_creator LOGIN PASSWORD %L \
                  CREATEROLE BYPASSRLS NOSUPERUSER', \
                 $1::text\
             )",
            &[&password],
        )
        .await
        .expect("format create role")
        .get(0);
    admin
        .batch_execute(&create_role)
        .await
        .expect("creator role");
    // CREATE DATABASE cannot run inside a multi-statement implicit transaction.
    admin
        .batch_execute("CREATE DATABASE mb_degraded OWNER mb_creator;")
        .await
        .expect("degraded database");

    let degraded_url = url_for(&url, "mb_creator", &password, "mb_degraded");
    megabase_auth::install_roles(&degraded_url)
        .await
        .expect("degraded install");

    let text = String::from_utf8(warnings.lock().expect("warnings").clone()).expect("utf8");
    for attribute in ["SUPERUSER", "CREATEDB", "REPLICATION"] {
        assert!(
            text.contains(&format!(
                "megabase: connecting role cannot grant {attribute} to supabase_admin"
            )),
            "{text}"
        );
    }
    assert!(
        !text.contains("cannot grant CREATEROLE to supabase_admin"),
        "CREATEROLE is within a CREATEROLE role's rights: {text}"
    );
    assert!(
        !text.contains("cannot grant BYPASSRLS to supabase_admin"),
        "BYPASSRLS is within this role's rights: {text}"
    );

    let degraded = attributes(&admin).await;
    let admin_row = degraded
        .iter()
        .find(|row| row.name == "supabase_admin")
        .expect("supabase_admin");
    assert!(!admin_row.superuser);
    assert!(admin_row.createrole);
    assert!(!admin_row.createdb);
    assert!(!admin_row.login);
    assert!(admin_row.inherit);
    assert!(!admin_row.replication);
    assert!(admin_row.bypassrls);
    for name in [
        "supabase_auth_admin",
        "supabase_storage_admin",
        "supabase_realtime_admin",
    ] {
        let row = degraded.iter().find(|row| row.name == name).expect(name);
        assert!(!row.login, "{name}");
        assert!(!row.inherit, "{name}");
        assert!(!row.replication, "{name}");
        assert!(!row.superuser, "{name}");
    }
    assert!(
        degraded
            .iter()
            .find(|row| row.name == "supabase_auth_admin")
            .expect("auth")
            .createrole
    );
    assert!(
        degraded
            .iter()
            .find(|row| row.name == "supabase_storage_admin")
            .expect("storage")
            .createrole
    );
    assert!(
        !degraded
            .iter()
            .find(|row| row.name == "supabase_realtime_admin")
            .expect("realtime")
            .createrole
    );

    cleanup(&admin).await;

    // A role the image already created keeps its attributes, including LOGIN.
    admin
        .batch_execute("CREATE ROLE supabase_auth_admin LOGIN NOINHERIT CREATEROLE NOREPLICATION;")
        .await
        .expect("pre-existing auth admin");
    let before = warnings.lock().expect("warnings").len();
    megabase_auth::install_roles(&url)
        .await
        .expect("superuser install");
    let after = String::from_utf8(warnings.lock().expect("warnings").clone()).expect("utf8");
    assert!(
        !after[before..].contains("cannot grant"),
        "a superuser install does not degrade: {}",
        &after[before..]
    );

    let created = attributes(&admin).await;
    let auth = created
        .iter()
        .find(|row| row.name == "supabase_auth_admin")
        .expect("auth");
    assert!(auth.login, "an existing LOGIN role is not altered");
    assert!(auth.createrole);
    assert!(!auth.inherit);

    let supabase_admin = created
        .iter()
        .find(|row| row.name == "supabase_admin")
        .expect("supabase_admin");
    assert!(!supabase_admin.login);
    assert!(supabase_admin.superuser);
    assert!(supabase_admin.createdb);
    assert!(supabase_admin.createrole);
    assert!(supabase_admin.inherit);
    assert!(supabase_admin.replication);
    assert!(supabase_admin.bypassrls);

    let realtime = created
        .iter()
        .find(|row| row.name == "supabase_realtime_admin")
        .expect("realtime");
    assert!(!realtime.login);
    assert!(!realtime.inherit);
    assert!(!realtime.replication);
    assert!(!realtime.createrole);

    megabase_auth::install_roles(&url)
        .await
        .expect("second install");
    let twice = attributes(&admin).await;
    assert_eq!(
        created, twice,
        "a second install does not change attributes"
    );

    admin
        .batch_execute(
            r#"
            CREATE TABLE public.widgets (id int);
            ALTER TABLE public.widgets ENABLE ROW LEVEL SECURITY;
            CREATE POLICY widgets_auth ON public.widgets
                FOR SELECT TO "supabase_auth_admin" USING (true);
            GRANT SELECT ON public.widgets TO "supabase_auth_admin";
            CREATE SCHEMA realtime AUTHORIZATION supabase_admin;
            GRANT CREATE, USAGE ON SCHEMA realtime TO supabase_realtime_admin;
            "#,
        )
        .await
        .expect("dump statements and realtime schema");

    let owner: String = admin
        .query_one(
            "SELECT nspowner::regrole::text FROM pg_namespace WHERE nspname = 'realtime'",
            &[],
        )
        .await
        .expect("owner")
        .get(0);
    assert_eq!(owner, "supabase_admin");

    cleanup(&admin).await;
    admin
        .batch_execute("DROP TABLE IF EXISTS public.widgets; DROP SCHEMA IF EXISTS realtime;")
        .await
        .expect("cleanup objects");
}

#[derive(Debug, PartialEq, Eq)]
struct RoleAttr {
    name: String,
    superuser: bool,
    createrole: bool,
    createdb: bool,
    login: bool,
    inherit: bool,
    replication: bool,
    bypassrls: bool,
}

async fn attributes(client: &tokio_postgres::Client) -> Vec<RoleAttr> {
    let rows = client
        .query(
            "SELECT rolname, rolsuper, rolcreaterole, rolcreatedb, rolcanlogin, \
                    rolinherit, rolreplication, rolbypassrls \
             FROM pg_roles \
             WHERE rolname IN ( \
                'supabase_admin', 'supabase_auth_admin', \
                'supabase_storage_admin', 'supabase_realtime_admin') \
             ORDER BY rolname",
            &[],
        )
        .await
        .expect("attributes");
    rows.into_iter()
        .map(|row| RoleAttr {
            name: row.get(0),
            superuser: row.get(1),
            createrole: row.get(2),
            createdb: row.get(3),
            login: row.get(4),
            inherit: row.get(5),
            replication: row.get(6),
            bypassrls: row.get(7),
        })
        .collect()
}

async fn cleanup(client: &tokio_postgres::Client) {
    // Terminate leftover sessions so DROP DATABASE is not blocked, and run
    // DROP DATABASE on its own: PostgreSQL refuses it inside a transaction.
    client
        .batch_execute(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE datname = 'mb_degraded' AND pid <> pg_backend_pid();",
        )
        .await
        .expect("terminate");
    client
        .batch_execute("DROP DATABASE IF EXISTS mb_degraded;")
        .await
        .expect("drop database");
    client
        .batch_execute(
            "DROP TABLE IF EXISTS public.widgets; DROP SCHEMA IF EXISTS realtime CASCADE;",
        )
        .await
        .expect("drop objects");
    // Members first (`mb_creator`, `authenticator`), then the roles they
    // belong to. `DROP OWNED` revokes grants so the role can be dropped.
    // This cluster is disposable: the test URL must not be a shared database.
    client
        .batch_execute(
            "DO $$
             DECLARE role_name text;
             BEGIN
                 FOREACH role_name IN ARRAY ARRAY[
                     'mb_creator', 'authenticator',
                     'supabase_admin', 'supabase_auth_admin',
                     'supabase_storage_admin', 'supabase_realtime_admin',
                     'anon', 'authenticated', 'service_role'
                 ]
                 LOOP
                     IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = role_name) THEN
                         EXECUTE format('DROP OWNED BY %I', role_name);
                         EXECUTE format('DROP ROLE %I', role_name);
                     END IF;
                 END LOOP;
             END $$;",
        )
        .await
        .expect("drop roles");
}

fn url_for(super_url: &str, user: &str, password: &str, database: &str) -> String {
    let config = super_url.parse::<tokio_postgres::Config>().expect("url");
    let host = match &config.get_hosts()[0] {
        tokio_postgres::config::Host::Tcp(host) => host.clone(),
        tokio_postgres::config::Host::Unix(_) => "localhost".to_string(),
    };
    let port = config.get_ports()[0];
    format!("postgres://{user}:{password}@{host}:{port}/{database}")
}

struct VecWriter(Arc<Mutex<Vec<u8>>>);

impl Write for VecWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("warnings").extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
