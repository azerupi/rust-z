#![feature(custom_attribute, custom_derive, plugin, try_from)]
#![plugin(serde_macros)]
#![allow(dead_code)]
#![feature(question_mark)]
#![feature(custom_derive)]
#![recursion_limit = "1024"]

#[macro_use]
extern crate error_chain;
extern crate clap;
extern crate yaml_rust as yaml;
extern crate url;
#[macro_use]
extern crate hyper;
extern crate chrono;
extern crate serde;
extern crate serde_json;
extern crate serde_yaml;
extern crate regex;
#[macro_use]
extern crate log;
extern crate env_logger;

use yaml::{YamlLoader, Yaml};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::io::Read;
use std::collections::BTreeMap;
use serde::{Serialize, Deserialize};
use std::fs;
use std::io::Write;

mod errors;
use errors::*;

mod crawl;
mod ponder;

mod gh {
    pub mod client;
    pub mod models;
    pub mod domain;
    pub mod http;
}

fn main() {
    let mut logger = env_logger::LogBuilder::new();
    logger.filter(None, log::LogLevelFilter::Info);
    logger.init().unwrap();

    if let Err(e) = main_() {
        error!("err: {}", e);
        for e in e.iter().skip(1) {
            error!("cause: {}", e);
        }
    }
}

fn main_() -> Result<()> {
    let config = read_args()?;

    match config {
        Config::Check => validate_plan()?,
        Config::Crawl => crawl::crawl()?,
        Config::Ponder => ponder::ponder()?,
        _ => panic!()
    }

    Ok(())
}

fn read_args() -> Result<Config> {
    use clap::*;

    let matches = App::new("Battleplan Rust Command Console")
        .subcommand(SubCommand::with_name("check"))
        .subcommand(SubCommand::with_name("crawl"))
        .subcommand(SubCommand::with_name("ponder"))
        .subcommand(SubCommand::with_name("compare"))
        .subcommand(SubCommand::with_name("merge"))
        .subcommand(SubCommand::with_name("triage"))
        .subcommand(SubCommand::with_name("discover"))
        .get_matches();

    match matches.subcommand_name() {
        Some("check") => Ok(Config::Check),
        Some("crawl") => Ok(Config::Crawl),
        Some("ponder") => Ok(Config::Ponder),
        Some("compare") => Ok(Config::Compare),
        Some("merge") => Ok(Config::Merge),
        Some(_) |
        None => Ok(Config::Check),
    }
}

enum Config {
    Check,
    Crawl,
    Ponder,
    Compare,
    Merge,
}

static DATA_DIR: &'static str = "./_data";

fn validate_plan() -> Result<()> {
    let plan = load_plan()?;

    plan.validate()
}

fn load_plan() -> Result<Battleplan> {
    let data_dir = PathBuf::from(DATA_DIR);
    let battlefronts = yaml_from_file(&data_dir.join("battlefronts.yml"))?;
    let campaigns = yaml_from_file(&data_dir.join("campaigns.yml"))?;
    let problems = yaml_from_file(&data_dir.join("problems.yml"))?;
    let teams = yaml_from_file(&data_dir.join("teams.yml"))?;
    let releases = yaml_from_file(&data_dir.join("releases.yml"))?;

    let battlefronts = battlefronts_from_yaml(battlefronts)?;
    let campaigns = campaigns_from_yaml(campaigns)?;
    let problems = problems_from_yaml(problems)?;
    let teams = teams_from_yaml(teams)?;
    let releases = releases_from_yaml(releases)?;

    Ok(Battleplan {
        battlefronts: battlefronts,
        campaigns: campaigns,
        problems: problems,
        teams: teams,
        releases: releases,
    })
}

fn yaml_from_file(path: &Path) -> Result<Vec<Yaml>> {
    let mut contents = String::new();
    File::open(path)?.read_to_string(&mut contents)?;
    Ok(YamlLoader::load_from_str(&contents)?)
}

struct Battleplan {
    battlefronts: Vec<Battlefront>,
    campaigns: Vec<Campaign>,
    problems: Vec<Problem>,
    teams: Vec<Team>,
    releases: Vec<Release>,
}

struct Battlefront {
    id: String,
    name: String,
    team: String,
    top: bool,
    pitch: String
}

struct Campaign {
    id: String,
    goal: String,
    pitch: String,
    top: bool,
    battlefront: String,
    tracking_link: String,
    release: String,
}

struct Problem {
    id: String,
    pitch: String,
    battlefront: String,
}

struct Team {
    id: String,
    name: String,
}

struct Release {
    id: String,
    future: bool,
}

macro_rules! verr {
    ($fmt:expr, $($arg:tt)*) => (warn!(concat!("validation error: ", $fmt), $($arg)*));
}

impl Battleplan {
    fn validate(&self) -> Result<()> {
        let mut good = true;

        for battlefront in &self.battlefronts {
            if !self.teams.iter().any(|x| x.id == battlefront.team) {
                good = false;
                verr!("battlefront {} mentions bogus team '{}'",
                      battlefront.id, battlefront.team);
            }
        }
        for campaign in &self.campaigns {
            if !self.battlefronts.iter().any(|x| x.id == campaign.battlefront) {
                good = false;
                verr!("campaign {} mentions bogus battlefront '{}'",
                      campaign.id, campaign.battlefront);
            }
            if !self.releases.iter().any(|x| x.id == campaign.release) {
                good = false;
                verr!("campaign {} mentions bogus release '{}'",
                      campaign.id, campaign.release);
            }

            if campaign.tracking_link.starts_with("http://") {
                verr!("campaign {} has https tracking link: {}",
                      campaign.id, campaign.tracking_link);
            }
        }
        for problem in &self.problems {
            if !self.battlefronts.iter().any(|x| x.id == problem.battlefront) {
                good = false;
                verr!("problem {} mentions bogus battlefront '{}'",
                      problem.id, problem.battlefront);
            }
        }

        if good {
            Ok(())
        } else {
            Err("invalid battleplan".into())
        }
    }
}


macro_rules! try_lookup_string {
    ($map: expr, $field_name:expr, $obj_type:expr, $obj_id:expr) => {{
        let field = lookup_string(&mut $map, $field_name);
        if let Err(e) = field {
            verr!("{} {}; {}", $obj_type, $obj_id, e);
            continue;
        }

        let field = field.expect("");

        field
    }}
}

macro_rules! try_lookup_bool {
    ($map: expr, $field_name:expr, $obj_type:expr, $obj_id:expr) => {{
        let field = lookup_bool(&mut $map, $field_name);
        if let Err(e) = field {
            verr!("{} {}; {}", $obj_type, $obj_id, e);
            continue;
        }
        let field = field.expect("");

        field
    }}
}

macro_rules! try_as_map {
    ($yaml: expr, $obj_type:expr, $obj_id:expr) => {{
        let map = $yaml.as_hash();
        if map.is_none() {
            verr!("{} {} is not a map", $obj_type, $obj_id);
            continue;
        }
        let map = map.expect("");

        map.clone()
    }}
}

fn lookup(y: &mut BTreeMap<Yaml, Yaml>, field_name: &str) -> Result<Yaml> {
    let key = Yaml::String(field_name.to_string());
    if let Some(y) = y.remove(&key) {
        Ok(y)
    } else {
        Err(format!("missing field `{}`", field_name).into())
    }
    
}

fn lookup_string(y: &mut BTreeMap<Yaml, Yaml>, field_name: &str) -> Result<String> {
    let y = lookup(y, field_name)?;
    if let Some(s) = y.as_str() {
        Ok(s.to_string())
    } else {
        Err("not a string".into())
    }
}

fn lookup_bool(y: &mut BTreeMap<Yaml, Yaml>, field_name: &str) -> Result<bool> {
    let y = lookup(y, field_name);
    // Fields that don't exist are false
    if y.is_err() { return Ok(false) };
    let y = y.expect("");

    match y {
        Yaml::Boolean(v) => {
            Ok(v)
        }
        _ => {
            Err("not a bool".into())
        }
    }
}

fn root_yaml_to_vec<'a>(y: &'a Vec<Yaml>, type_: &str) -> Result<&'a Vec<Yaml>> {
    let y = y.get(0)
        .ok_or(Error::from(format!("{} yaml has no elements", type_)))?;
    let y = y.as_vec()
        .ok_or(Error::from(format!("{} yaml is not an array", type_)))?;

    Ok(y)
}

fn warn_extra_fields(y: BTreeMap<Yaml, Yaml>, type_: &str, id: &str) {
    for (key, _) in y.into_iter() {
        verr!("{} {} has extra field: {:?}", type_, id, key);
    }
}

fn battlefronts_from_yaml(y: Vec<Yaml>) -> Result<Vec<Battlefront>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "battlefront")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "battlefront", i);

        let id = try_lookup_string!(map, "id", "battlefront", i);
        let name = try_lookup_string!(map, "name", "battlefront", id);
        let team = try_lookup_string!(map, "team", "battlefront", id);
        let top = try_lookup_bool!(map, "top", "battlefront", id);
        let pitch = try_lookup_string!(map, "pitch", "battlefront", id);

        warn_extra_fields(map, "battlefront", &id);

        res.push(Battlefront {
            id: id,
            name: name,
            team: team,
            top: top,
            pitch: pitch,
        });
    }

    Ok(res)
}

fn campaigns_from_yaml(y: Vec<Yaml>) -> Result<Vec<Campaign>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "campaign")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "campaign", i);

        let id = try_lookup_string!(map, "id", "campaign", i);
        let goal = try_lookup_string!(map, "goal", "campaign", id);
        let top = try_lookup_bool!(map, "top", "campaign", id);
        let pitch = try_lookup_string!(map, "pitch", "campaign", id);
        let battlefront = try_lookup_string!(map, "battlefront", "campaign", id);
        let tracking_link = try_lookup_string!(map, "tracking-link", "campaign", id);
        let release = try_lookup_string!(map, "release", "campaign", id);

        warn_extra_fields(map, "campaign", &id);

        res.push(Campaign {
            id: id,
            goal: goal,
            top: top,
            pitch: pitch,
            battlefront: battlefront,
            tracking_link: tracking_link,
            release: release,
        });
    }

    Ok(res)
}

fn problems_from_yaml(y: Vec<Yaml>) -> Result<Vec<Problem>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "problem")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "problem", i);

        let id = try_lookup_string!(map, "id", "problem", i);
        let pitch = try_lookup_string!(map, "pitch", "problem", id);
        let battlefront = try_lookup_string!(map, "battlefront", "problem", id);

        warn_extra_fields(map, "problem", &id);

        res.push(Problem {
            id: id,
            pitch: pitch,
            battlefront: battlefront,
        });
    }

    Ok(res)
}

fn teams_from_yaml(y: Vec<Yaml>) -> Result<Vec<Team>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "team")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "team", i);

        let id = try_lookup_string!(map, "id", "team", i);
        let name = try_lookup_string!(map, "name", "team", id);

        warn_extra_fields(map, "team", &id);

        res.push(Team {
            id: id,
            name: name,
        });
    }

    Ok(res)
}

fn releases_from_yaml(y: Vec<Yaml>) -> Result<Vec<Release>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "release")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "release", i);

        let id = try_lookup_string!(map, "id", "release", i);
        let future = try_lookup_bool!(map, "future", "release", id);

        warn_extra_fields(map, "release", &id);

        res.push(Release {
            id: id,
            future: future,
        });
    }

    Ok(res)
}

fn write_yaml<T>(name: &str, value: T) -> Result<()>
    where T: Serialize
{
    let data_s = serde_yaml::to_string(&value)
        .chain_err(|| format!("encoding yaml for {}", name))?;

    let data_file = &PathBuf::from(DATA_DIR).join(format!("gen/{}.yml", name));
    let data_dir = data_file.parent().expect("");
    fs::create_dir_all(data_dir)?;
    let mut f = File::create(data_file)?;
    writeln!(f, "{}", data_s)?;

    info!("{} updated", data_file.display());

    Ok(())
}

fn load_yaml<T>(name: &str) -> Result<T>
    where T: Deserialize
{
    let data_file = &PathBuf::from(DATA_DIR).join(format!("gen/{}.yml", name));
    let mut file = File::open(data_file)?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;

    // HACK: the yaml deserializer sees " ... " as some kind of invalid
    // "document indicator". Remove it.
    let buf = buf.replace(" ... ", " .. ");

    let value = serde_yaml::from_str(&buf)
        .chain_err(|| format!("decoding yaml for {}", name))?;

    Ok(value)
}
