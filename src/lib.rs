mod admin_table;
mod room_resolver;
mod wasm;

use anyhow::{Context, bail};
use matrix_sdk::{
    config::SyncSettings,
    event_handler::Ctx,
    matrix_auth::{MatrixAuth, MatrixSession, MatrixSessionTokens, LoginBuilder},
    room::Room,
    RoomState,
    ruma::{
        api::client::session::get_login_types::v3::{IdentityProvider, LoginType},
        events::{
            key::verification::{request::ToDeviceKeyVerificationRequestEvent, VerificationMethod},
            reaction::ReactionEventContent,
            relation::Annotation,
            room::{
                member::StrippedRoomMemberEvent,
                message::{MessageType, RoomMessageEventContent, SyncRoomMessageEvent},
            },
        },
        presence::PresenceState,
        OwnedUserId, RoomId, UserId,
    },
    encryption::verification::{Emoji, SasState, SasVerification, Verification, VerificationRequest, VerificationRequestState},
    Client,
};
use matrix_sdk_base::SessionMeta;
use notify::{RecursiveMode, Watcher};
use room_resolver::RoomResolver;
use serde::Deserialize;
use std::{collections::HashMap, env, fs, net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
    time::{sleep, Duration},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{FramedRead, LinesCodec};
use tracing::{debug, error, info, trace, warn};
use wasm::{GuestState, Module, WasmModules};

use crate::admin_table::DEVICE_ID_ENTRY;

/// The configuration to run a trinity instance with.
#[derive(Deserialize)]
pub struct BotConfig {
    /// the matrix homeserver the bot should connect to.
    pub home_server: Option<String>,
    /// the user_id to be used on the homeserver.
    pub user_id: String,
    /// password to be used to log into the homeserver.
    pub password: Option<String>,
    /// access_token to borrow a login made through some other means
    pub access_token: Option<String>,
    /// device_id is required if using the access_token, though it
    /// can also come from the db.
    pub device_id: Option<String>,
    /// where to store the matrix-sdk internal data.
    pub matrix_store_path: String,
    /// where to store the additional database data.
    pub redb_path: String,
    /// the admin user id for the bot.
    pub admin_user_id: OwnedUserId,
    /// paths where modules can be loaded.
    pub modules_paths: Vec<PathBuf>,
    /// module specific configuration to forward to corresponding handler.
    pub modules_config: Option<HashMap<String, HashMap<String, String>>>,
}

impl BotConfig {
    /// Generate a `BotConfig` from a TOML config file.
    ///
    /// If `path` matches `None`, will search for a file called `config.toml` in an XDG
    /// compliant configuration directory (e.g ~/.config/trinity/config.toml on Linux).
    pub fn from_config(path: Option<String>) -> anyhow::Result<Self> {
        let config_path = match path {
            Some(a) => a,
            None => {
                let dirs = directories::ProjectDirs::from("", "", "trinity")
                    .context("config file not found")?;
                let path = dirs.config_dir().join("config.toml");
                String::from(path.to_str().unwrap())
            }
        };
        let contents = fs::read_to_string(&config_path)?;
        let config: BotConfig = toml::from_str(&contents)?;

        debug!("Using configuration from {config_path}");
        Ok(config)
    }

    /// Generate a `BotConfig` from the process' environment.
    pub fn from_env() -> anyhow::Result<Self> {
        // override environment variables with contents of .env file, unless they were already set
        // explicitly.
        dotenvy::dotenv().ok();

        let home_server = env::var("HOMESERVER").context("missing HOMESERVER variable")?;
        let user_id = env::var("BOT_USER_ID").context("missing bot user id in BOT_USER_ID")?;
        let password = env::var("BOT_PWD").context("missing bot user id in BOT_PWD")?;
        let matrix_store_path =
            env::var("MATRIX_STORE_PATH").context("missing MATRIX_STORE_PATH")?;
        let redb_path = env::var("REDB_PATH").context("missing REDB_PATH")?;

        let admin_user_id =
            env::var("ADMIN_USER_ID").context("missing admin user id in ADMIN_USER_ID")?;
        let admin_user_id = admin_user_id
            .try_into()
            .context("impossible to parse admin user id")?;

        // Read the module paths (separated by commas), check they exist, and return the whole
        // list.
        let modules_paths = env::var("MODULES_PATHS")
            .as_deref()
            .unwrap_or("./modules/target/wasm32-unknown-unknown/release")
            .split(',')
            .map(|path| {
                let path = PathBuf::from(path);
                anyhow::ensure!(
                    path.exists(),
                    "{} doesn't reference a valid path",
                    path.to_string_lossy()
                );
                Ok(path)
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .context("a module path isn't valid")?;

        debug!("Using configuration from environment");
        Ok(Self {
            home_server: Some(home_server),
            user_id,
            password: Some(password),
            access_token: None,
            device_id: None,
            matrix_store_path,
            admin_user_id,
            redb_path,
            modules_paths,
            modules_config: None,
        })
    }
}

struct AuthInfo<'a> {
    _config: &'a BotConfig,
    /// used for SSO authentication
    login_token: String,
}

pub(crate) type ShareableDatabase = Arc<redb::Database>;

struct AppCtx {
    modules: WasmModules,
    modules_paths: Vec<PathBuf>,
    modules_config: HashMap<String, HashMap<String, String>>,
    needs_recompile: bool,
    admin_user_id: OwnedUserId,
    db: ShareableDatabase,
    room_resolver: RoomResolver,
}

impl AppCtx {
    /// Create a new `AppCtx`.
    ///
    /// Must be called from a blocking context.
    pub fn new(
        client: Client,
        modules_paths: Vec<PathBuf>,
        modules_config: HashMap<String, HashMap<String, String>>,
        db: ShareableDatabase,
        admin_user_id: OwnedUserId,
    ) -> anyhow::Result<Self> {
        let room_resolver = RoomResolver::new(client);
        Ok(Self {
            modules: WasmModules::new(db.clone(), &modules_paths, &modules_config)?,
            modules_paths,
            modules_config,
            needs_recompile: false,
            admin_user_id,
            db,
            room_resolver,
        })
    }

    pub async fn set_needs_recompile(ptr: Arc<Mutex<Self>>) {
        {
            let need = &mut ptr.lock().await.needs_recompile;
            if *need {
                return;
            }
            *need = true;
        }

        tokio::task::spawn_blocking(move || {
            let mut ptr = futures::executor::block_on(async {
                tokio::time::sleep(Duration::new(1, 0)).await;
                ptr.lock().await
            });

            match WasmModules::new(ptr.db.clone(), &ptr.modules_paths, &ptr.modules_config) {
                Ok(modules) => {
                    ptr.modules = modules;
                    info!("successful hot reload!");
                }
                Err(err) => {
                    error!("hot reload failed: {err:#}");
                }
            }

            ptr.needs_recompile = false;
        });
    }
}

#[derive(Clone)]
struct App {
    inner: Arc<Mutex<AppCtx>>,
}

impl App {
    pub fn new(ctx: AppCtx) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ctx)),
        }
    }
}

/// Try to handle a message assuming it's an `!admin` command.
fn try_handle_admin<'a>(
    content: &str,
    sender: &UserId,
    room: &RoomId,
    store: &mut wasmtime::Store<GuestState>,
    modules: impl Clone + Iterator<Item = &'a Module>,
    room_resolver: &mut RoomResolver,
) -> Option<Vec<wasm::Action>> {
    let rest = content.strip_prefix("!admin")?;

    trace!("trying admin for {content}");

    if let Some(rest) = rest.strip_prefix(' ') {
        let rest = rest.trim();
        if let Some((module, rest)) = rest.split_once(' ').map(|(l, r)| (l, r.trim())) {
            // If the next word resolves to a valid room id use that, otherwise use the
            // current room.
            let (possible_room, rest) = rest
                .split_once(' ')
                .map_or((rest, ""), |(l, r)| (l, r.trim()));

            let (target_room, rest) = match room_resolver.resolve_room(possible_room) {
                Ok(Some(resolved_room)) => (resolved_room, rest.to_string()),
                Ok(None) | Err(_) => (room.to_string(), format!("{} {}", possible_room, rest)),
            };

            let mut found = None;
            for m in modules {
                if m.name() == module {
                    found = match m.admin(&mut *store, rest.trim(), sender, target_room.as_str()) {
                        Ok(actions) => Some(actions),
                        Err(err) => {
                            error!("error when handling admin command: {err:#}");
                            None
                        }
                    };
                    break;
                }
            }
            found
        } else {
            Some(vec![wasm::Action::Respond(wasm::Message {
                text: "missing command".to_owned(),
                html: None,
                to: sender.to_string(),
            })])
        }
    } else {
        Some(vec![wasm::Action::Respond(wasm::Message {
            text: "missing module and command".to_owned(),
            html: None,
            to: sender.to_string(),
        })])
    }
}

fn try_handle_help<'a>(
    content: &str,
    sender: &UserId,
    store: &mut wasmtime::Store<GuestState>,
    modules: impl Clone + Iterator<Item = &'a Module>,
) -> Option<wasm::Action> {
    let rest = content.strip_prefix("!help")?;

    // Special handling for help messages.
    let (msg, html) = if rest.trim().is_empty() {
        let mut msg = String::from("Available modules:");
        let mut html = String::from("Available modules: <ul>");
        for m in modules {
            let help = match m.help(&mut *store, None) {
                Ok(msg) => Some(msg),
                Err(err) => {
                    error!("error when handling help command: {err:#}");
                    None
                }
            }
            .unwrap_or("<missing>".to_string());

            msg.push_str(&format!("\n- {name}: {help}", name = m.name(), help = help));
            // TODO lol sanitize html
            html.push_str(&format!(
                "<li><b>{name}</b>: {help}</li>",
                name = m.name(),
                help = help
            ));
        }
        html.push_str("</ul>");

        (msg, html)
    } else if let Some(rest) = rest.strip_prefix(' ') {
        let rest = rest.trim();
        let (module, topic) = rest
            .split_once(' ')
            .map(|(l, r)| (l, Some(r.trim())))
            .unwrap_or((rest, None));
        let mut found = None;
        for m in modules {
            if m.name() == module {
                found = m.help(&mut *store, topic).ok();
                break;
            }
        }
        let msg = if let Some(content) = found {
            content
        } else {
            format!("module {module} not found")
        };
        (msg.clone(), msg)
    } else {
        return None;
    };

    Some(wasm::Action::Respond(wasm::Message {
        text: msg,
        html: Some(html),
        to: sender.to_string(), // TODO rather room?
    }))
}

enum AnyEvent {
    RoomMessage(RoomMessageEventContent),
    Reaction(ReactionEventContent),
}

impl AnyEvent {
    async fn send(self, room: &mut Room) -> anyhow::Result<()> {
        let _ = match self {
            AnyEvent::RoomMessage(e) => room.send(e).await?,
            AnyEvent::Reaction(e) => room.send(e).await?,
        };
        Ok(())
    }
}

async fn on_verification_request(ev: ToDeviceKeyVerificationRequestEvent, client: Client) -> anyhow::Result<()> {
    let request = client
        .encryption()
        .get_verification_request(&ev.sender, &ev.content.transaction_id)
        .await
        .expect("Request object wasn't created");
    if !request.is_self_verification() {
        debug!("Only self-verification supported for now");
        return Ok(());
    }

    tokio::spawn(request_verification_handler(client, request));
    Ok(())
}

async fn request_verification_handler(client: Client, request: VerificationRequest) -> anyhow::Result<()> {
    println!("Accepting verification request from {} (me)", request.other_user_id(),);
    request.accept().await?; // Now the craziness starts...

    println!("Supported methods: {:?}", request.their_supported_methods());
    if let Some(methods) = request.their_supported_methods() {
        if ! methods.contains(&VerificationMethod::SasV1) {
            bail!("Only SasV1 supported for now");
        }
    } else {
        bail!("No verification methods supported??!");
    }

    let mut stream = request.changes();
    while let Some(state) = stream.next().await {
        match state {
            VerificationRequestState::Created { .. }
            | VerificationRequestState::Requested { .. }
            | VerificationRequestState::Ready { .. } => (),
            VerificationRequestState::Transitioned { verification } => {
                if let Verification::SasV1(s) = verification {
                    tokio::spawn(sas_verification_handler(client, s));
                    break;
                }
            },
            VerificationRequestState::Done | VerificationRequestState::Cancelled(_) => break,
        }
    }

    Ok(())
}

async fn sas_verification_handler(_client: Client, sas: SasVerification) -> anyhow::Result<()> {
    println!("Starting verification");
    sas.accept().await?;
    let mut stream = sas.changes();

    while let Some(state) = stream.next().await {
        if let SasState::KeysExchanged{emojis, decimals: _} = state {
            tokio::spawn(wait_for_confirmation(sas.clone(), emojis.unwrap().emojis));
        } else if let SasState::Done{ .. } = state {
            println!("Successfully verified: {:?}", sas.other_device().local_trust_state());
            return Ok(());
        } else {
            println!("Other state: {:?}", state);
        }
    }

    bail!("Sas verification seems to have failed?");
}

// Ugh, this isn't great. It asks whether the verification emoji match, using stdin.
// Which means it will get buried in the logging output, and it's kind of a weird way
// to provide confirmation. I'm not sure what a better way is, though.
//
// The code here is very clunky too, but I'm not inclined to clean it up when I really want to replace it entirely.
async fn wait_for_confirmation(sas: SasVerification, emoji: [Emoji; 7]) -> anyhow::Result<()> {
    println!("Verification emoji: {}", emoji.map(|e| format!("{}{}", e.symbol, e.description)).join(" "));

    print!("Does it match (y/n)? ");
    tokio::io::stdout().flush().await?;

    let stdin = tokio::io::stdin();
    let mut reader = FramedRead::new(stdin, LinesCodec::new());
    if let Some(line) = reader.next().await {
        let line = line.expect("unable to decode");
        if line == "y" {
            sas.confirm().await.expect("confirmation failed");
        } else {
            sas.cancel().await.expect("cancellation failed");
        }
    }
    Ok(())
}

async fn on_message(
    ev: SyncRoomMessageEvent,
    mut room: Room,
    client: Client,
    Ctx(ctx): Ctx<App>,
) -> anyhow::Result<()> {
    if room.state() != RoomState::Joined {
        // Ignore non-joined rooms events.
        return Ok(());
    }

    if ev.sender() == client.user_id().unwrap() {
        // Skip messages sent by the bot.
        return Ok(());
    }

    if ev.as_original().is_none() {
        trace!("redacted message");
        return Ok(());
    }

    let unredacted = ev.as_original().unwrap();

    let content = if let MessageType::Text(text) = &unredacted.content.msgtype {
        text.body.to_string()
    } else {
        // Ignore other kinds of messages at the moment.
        return Ok(());
    };

    // TEMPORARY: Switch back to trace!
    info!(
        "Received a message from {} in {}: {}",
        ev.sender(),
        room.display_name().await.unwrap(),
        content,
    );

    if content.contains("you are a good boy") {
        let reaction = ReactionEventContent::new(Annotation::new(ev.event_id().to_owned(), "👀".to_owned()));
        room.send(reaction).await?;
        let message = RoomMessageEventContent::text_html("thank you", "thank <a href='htts://aapx.org/'>you</a>");
        room.send(message).await?;
    }

    // TODO ohnoes, locking across other awaits is bad
    // TODO Use a lock-free data-structure for the list of modules + put locks in the module
    // internal implementation?
    // TODO or create a new wasm instance per message \o/
    let ctx = ctx.inner.clone();
    let room_id = room.room_id().to_owned();

    let event_id = ev.event_id().to_owned();

    let new_actions = tokio::task::spawn_blocking(move || {
        let ctx = &mut *futures::executor::block_on(ctx.lock());

        let (store, modules) = ctx.modules.iter();

        if ev.sender() == ctx.admin_user_id {
            match try_handle_admin(
                &content,
                &ctx.admin_user_id,
                &room_id,
                store,
                modules.clone(),
                &mut ctx.room_resolver,
            ) {
                None => {}
                Some(actions) => {
                    trace!("handled by admin, skipping modules");
                    return actions;
                }
            }
        }

        if let Some(actions) = try_handle_help(&content, ev.sender(), store, modules.clone()) {
            trace!("handled by help, skipping modules");
            return vec![actions];
        }

        for module in modules {
            trace!("trying to handle message with {}...", module.name());
            match module.handle(&mut *store, &content, ev.sender(), &room_id) {
                Ok(actions) => {
                    if !actions.is_empty() {
                        // TODO support handling the same message with several handlers.
                        trace!("{} returned a response!", module.name());
                        return actions;
                    }
                }
                Err(err) => {
                    warn!("wasm module {} ran into an error: {err}", module.name());
                }
            }
        }

        Vec::new()
    })
    .await?;

    let new_events = new_actions
        .into_iter()
        .map(|a| match a {
            wasm::Action::Respond(msg) => {
                let content = if let Some(html) = msg.html {
                    RoomMessageEventContent::text_html(msg.text, html)
                } else {
                    RoomMessageEventContent::text_plain(msg.text)
                };
                AnyEvent::RoomMessage(content)
            }
            wasm::Action::React(reaction) => {
                let reaction =
                    ReactionEventContent::new(Annotation::new(event_id.clone(), reaction));
                AnyEvent::Reaction(reaction)
            }
        })
        .collect::<Vec<_>>();

    for event in new_events {
        event.send(&mut room).await?;
    }

    Ok(())
}

/// Autojoin mixin.
async fn on_stripped_state_member(
    room_member: StrippedRoomMemberEvent,
    client: Client,
    room: Room,
) {
    if room_member.state_key != client.user_id().unwrap() {
        // the invite we've seen isn't for us, but for someone else. ignore
        return;
    }

    // looks like the room is an invited room, let's attempt to join then
    if room.state() == RoomState::Invited {
        // The event handlers are called before the next sync begins, but
        // methods that change the state of a room (joining, leaving a room)
        // wait for the sync to return the new room state so we need to spawn
        // a new task for them.
        tokio::spawn(async move {
            debug!("Autojoining room {}", room.room_id());
            let mut delay = 1;

            while let Err(err) = room.join().await {
                // retry autojoin due to synapse sending invites, before the
                // invited user can join for more information see
                // https://github.com/matrix-org/synapse/issues/4345
                warn!(
                    "Failed to join room {} ({err:?}), retrying in {delay}s",
                    room.room_id()
                );

                sleep(Duration::from_secs(delay)).await;
                delay *= 2;

                if delay > 3600 {
                    error!("Can't join room {} ({err:?})", room.room_id());
                    break;
                }
            }

            debug!("Successfully joined room {}", room.room_id());
        });
    }
}

async fn login_with_password<'a>(config: &'a BotConfig, client: &Client)
                                 -> Result<LoginBuilder, anyhow::Error>
{
    println!("Logging in with username and password...");
    let Some(password) = &config.password else { bail!("password required") };
    Ok(
        client.matrix_auth().login_username(
            &config.user_id,
            password,
        ).initial_device_display_name("my initial device display name (TODO)")
    )
}

async fn login_with_sso<'a>(
    info: &'a mut AuthInfo<'a>,
    auth: &MatrixAuth,
    idp: Option<&IdentityProvider>
) -> Result<LoginBuilder, anyhow::Error>
{
    let addr = SocketAddr::from(([0, 0, 0, 0], 43210));
    let listener = TcpListener::bind(&addr).await?;
    println!("Listening on: http://{}", addr);

    let sso_url = auth.get_sso_login_url(
        &format!("http://localhost:{}/callback", addr.port()),
        idp.map(|p| p.id.as_str())
    ).await;

    if let Some(prov) = idp {
        println!("using id provider {}", prov.name);
    }

    println!("\nOpen this URL in your browser: {}", sso_url.unwrap());

    let mut token: Result<String, anyhow::Error>;
    loop {
        println!("accepting...");
        let (mut stream, _) = listener.accept().await?;

        token = tokio::task::spawn(async move {
            let mut buffer = [0; 1024];
            let nread = stream.read(&mut buffer).await?;
            println!("read {} bytes", nread);
            let Some(first_newline) = buffer[..nread].iter().position(|&c| c == 10)
            else { bail!("Invalid request (short)") };
            let data = std::str::from_utf8(&buffer[..first_newline])?;
            let Some(mut start) = data.find("?loginToken=")
            else { bail!("Invalid request (no token)"); };
            start += 12;
            let Some(mut end) = data[start..].find(" ")
            else { bail!("Invalid request (no space)") };
            end += start;
            let token = String::from(&data[start..end]);

            let contents = "<h1>Logging in</h1><p>You may close this page.";
            let content_length = contents.len();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {content_length}\r\n\r\n{contents}");
            let _ = stream.write_all(response.as_bytes()).await;
            Ok(token)
        }).await?;

        if token.is_ok() { break }
        println!("error = {}", token.unwrap_err());
    }

    info.login_token = token.unwrap();
    Ok(auth.login_token(&info.login_token))
}

/// Run the client for the given `BotConfig`.
pub async fn run(config: BotConfig) -> anyhow::Result<()> {
    let user_id = UserId::parse(config.user_id.clone())?;
    let base_dir = if let Some(dir) = dirs::data_dir() {
        dir
    } else if let Ok(dir) = std::env::current_dir() {
        dir
    } else {
        PathBuf::from(".")
    };
    let store_path = base_dir.join(&config.matrix_store_path);
    let redb_path = base_dir.join(&config.redb_path);

    let store = matrix_sdk_sqlite::make_store_config(&store_path, None).await?;
    let client = Client::builder()
        .server_name(user_id.server_name())
        .store_config(store)
        .build()
        .await?;

    // Create the database, and try to find a device id.
    let db = Arc::new(unsafe { redb::Database::create(redb_path, 1024 * 1024)? });

    // First we need to log in.
    debug!("logging in...");
    let login_types = client.matrix_auth().get_login_types().await?.flows;
    debug!("login types supported by server: {login_types:?}");

    let mut info = AuthInfo { _config: &config, login_token: String::from("") };
    let mut login_builder = None;
    if config.access_token.is_none() {
        for login_type in login_types {
            match login_type {
                LoginType::Password(_) => {
                    if config.password.is_some() {
                        login_builder = login_with_password(&config, &client).await.ok(); // FIXME
                        break
                    }
                },
                LoginType::Sso(ref sso) => {
                    login_builder =
                        match sso.identity_providers.len() {
                            0 => login_with_sso(&mut info, &client.matrix_auth(), None).await.ok(), // FIXME
                            1 => login_with_sso(&mut info, &client.matrix_auth(), Some(&sso.identity_providers[0])).await.ok(), // FIXME
                            _ => panic!("TODO: Multiple identity providers"),
                        };
                    break;
                },
                LoginType::Token(_) => {}, // Used for SSO
                _ => {},
            }
        }

        if login_builder.is_none() {
            bail!("Login failed!");
        };
    }

    let mut db_device_id = None;
    if let Some(device_id) = admin_table::read_str(&db, DEVICE_ID_ENTRY)
        .context("reading device_id from the database")?
    {
        trace!("reusing previous device_id...");
        // the login builder keeps a reference to the previous device id string, so can't clone
        // db_device_id here, it has to outlive the login_builder.
        db_device_id = Some(device_id);
        if let Some(lb) = login_builder {
            login_builder = Some(lb.device_id(db_device_id.as_ref().unwrap()));
        }
    }

    let device_id = if let Some(login_builder) = login_builder {
        let resp = login_builder.send().await?;
        resp.device_id.to_string()
    } else if let Some(id) = config.device_id {
        id
    } else {
        bail!("device_id required for access_token login")
    };

    if db_device_id.as_ref() != Some(&device_id) {
        match db_device_id {
            Some(prev) => {
                warn!("overriding device_id (previous was {prev}, new is {device_id})")
            }
            None => debug!("storing new device_id for the first time..."),
        }
        admin_table::write_str(&db, DEVICE_ID_ENTRY, &device_id)
            .context("writing new device_id into the database")?;
    }

    if config.access_token.is_some() {
        let session = MatrixSession {
            meta: SessionMeta {
                user_id,
                device_id: device_id.into(),
            },
            tokens: MatrixSessionTokens {
                access_token: config.access_token.unwrap(),
                refresh_token: None,
            }
        };
        client.restore_session(session).await?;
    }

    let modules_config = config.modules_config.unwrap_or_default();

    client
        .user_id()
        .context("impossible state: missing user id for the logged in bot?")?;

    // An initial sync to set up state and so our bot doesn't respond to old
    // messages. If the `StateStore` finds saved state in the location given the
    // initial sync will be skipped in favor of loading state from the store
    debug!("starting initial sync...");
    let mut sync_settings = SyncSettings::default();
    if let Some(sync_token) = client.store().get_custom_value(b"hacky-session-persistence").await? {
        sync_settings = sync_settings.token(String::from_utf8_lossy(&sync_token));
    }
    loop {
        match client.sync_once(sync_settings.clone()).await {
            Ok(response) => {
                let sync_token = response.next_batch;
                sync_settings = sync_settings.token(sync_token.clone());
                client.store().set_custom_value(b"hacky-session-persistence", sync_token.into()).await?;
                break;
            }
            Err(error) => {
                println!("error during initial sync: {error}");
                println!("retrying...");
            }
        }
    }

    debug!("setting up app...");
    let client_copy = client.clone();
    let app_ctx = tokio::task::spawn_blocking(|| {
        AppCtx::new(
            client_copy,
            config.modules_paths,
            modules_config,
            db,
            config.admin_user_id,
        )
    })
    .await??;
    let app = App::new(app_ctx);

    let _watcher_guard = watcher(app.inner.clone()).await?;

    println!("ACCESS TOKEN FOR SKIPPING LOGIN WHEN RESTARTING (put this in config.toml): {:?}", client.access_token().unwrap());

    debug!("setup ready! now listening to incoming messages.");
    client.add_event_handler_context(app);
    client.add_event_handler(on_message);
    client.add_event_handler(on_stripped_state_member);
    client.add_event_handler(on_verification_request);

    // Note: this method will never return.
    client.sync(sync_settings.clone()).await?;

    tokio::select! {
        _ = handle_signals() => {
            // Exit :)
        }

        Err(err) = client.sync(sync_settings) => {
            anyhow::bail!(err);
        }
    }

    // Set bot presence to offline.
    let request = matrix_sdk::ruma::api::client::presence::set_presence::v3::Request::new(
        client.user_id().unwrap().to_owned(),
        PresenceState::Offline,
    );

    client.send(request, None).await?;

    info!("properly exited, have a nice day!");
    Ok(())
}

async fn handle_signals() -> anyhow::Result<()> {
    //use futures::StreamExt as _;
    use signal_hook::consts::signal::*;
    use signal_hook_tokio::*;

    let mut signals = Signals::new([SIGINT, SIGHUP, SIGQUIT, SIGTERM])?;
    let handle = signals.handle();

    while let Some(signal) = signals.next().await {
        match signal {
            SIGINT | SIGHUP | SIGQUIT | SIGTERM => {
                handle.close();
                break;
            }
            _ => {
                // Don't care.
            }
        }
    }

    Ok(())
}

async fn watcher(app: Arc<Mutex<AppCtx>>) -> anyhow::Result<Vec<notify::RecommendedWatcher>> {
    let modules_paths = { app.lock().await.modules_paths.clone() };

    let mut watchers = Vec::with_capacity(modules_paths.len());
    for modules_path in modules_paths {
        debug!(
            "setting up watcher on @ {}...",
            modules_path.to_string_lossy()
        );

        let rt_handle = tokio::runtime::Handle::current();
        let app = app.clone();
        let mut watcher = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| match res {
                Ok(event) => {
                    // Only watch wasm files
                    if !event.paths.iter().any(|path| {
                        if let Some(ext) = path.extension() {
                            ext == "wasm"
                        } else {
                            false
                        }
                    }) {
                        return;
                    }

                    match event.kind {
                        notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_) => {
                            // Trigger an update of the modules.
                            let app = app.clone();
                            rt_handle.spawn(async move {
                                AppCtx::set_needs_recompile(app).await;
                            });
                        }
                        notify::EventKind::Access(_)
                        | notify::EventKind::Any
                        | notify::EventKind::Other => {}
                    }
                }
                Err(e) => warn!("watch error: {e:?}"),
            },
        )?;

        watcher.watch(&modules_path, RecursiveMode::Recursive)?;
        watchers.push(watcher);
    }

    debug!("watcher setup done!");
    Ok(watchers)
}
