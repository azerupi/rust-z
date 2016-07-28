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
    let themes = yaml_from_file(&data_dir.join("themes.yml"))?;
    let goals = yaml_from_file(&data_dir.join("goals.yml"))?;
    let problems = yaml_from_file(&data_dir.join("problems.yml"))?;
    let teams = yaml_from_file(&data_dir.join("teams.yml"))?;
    let releases = yaml_from_file(&data_dir.join("releases.yml"))?;

    let themes = themes_from_yaml(themes)?;
    let goals = goals_from_yaml(goals)?;
    let problems = problems_from_yaml(problems)?;
    let teams = teams_from_yaml(teams)?;
    let releases = releases_from_yaml(releases)?;

    Ok(Battleplan {
        themes: themes,
        goals: goals,
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
    themes: Vec<Theme>,
    goals: Vec<Goal>,
    problems: Vec<Problem>,
    teams: Vec<Team>,
    releases: Vec<Release>,
}

struct Theme {
    id: String,
    name: String,
    team: String,
    top: bool,
    pitch: String
}

struct Goal {
    id: String,
    goal: String,
    pitch: String,
    top: bool,
    theme: String,
    tracking_link: String,
    release: String,
}

struct Problem {
    id: String,
    pitch: String,
    theme: String,
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

        for theme in &self.themes {
            if !self.teams.iter().any(|x| x.id == theme.team) {
                good = false;
                verr!("theme {} mentions bogus team '{}'",
                      theme.id, theme.team);
            }
        }
        for goal in &self.goals {
            if !self.themes.iter().any(|x| x.id == goal.theme) {
                good = false;
                verr!("goal {} mentions bogus theme '{}'",
                      goal.id, goal.theme);
            }
            if !self.releases.iter().any(|x| x.id == goal.release) {
                good = false;
                verr!("goal {} mentions bogus release '{}'",
                      goal.id, goal.release);
            }

            if goal.tracking_link.starts_with("http://") {
                verr!("goal {} has https tracking link: {}",
                      goal.id, goal.tracking_link);
            }
        }
        for problem in &self.problems {
            if !self.themes.iter().any(|x| x.id == problem.theme) {
                good = false;
                verr!("problem {} mentions bogus theme '{}'",
                      problem.id, problem.theme);
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

fn themes_from_yaml(y: Vec<Yaml>) -> Result<Vec<Theme>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "theme")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "theme", i);

        let id = try_lookup_string!(map, "id", "theme", i);
        let name = try_lookup_string!(map, "name", "theme", id);
        let team = try_lookup_string!(map, "team", "theme", id);
        let top = try_lookup_bool!(map, "top", "theme", id);
        let pitch = try_lookup_string!(map, "pitch", "theme", id);

        warn_extra_fields(map, "theme", &id);

        res.push(Theme {
            id: id,
            name: name,
            team: team,
            top: top,
            pitch: pitch,
        });
    }

    Ok(res)
}

fn goals_from_yaml(y: Vec<Yaml>) -> Result<Vec<Goal>> {
    let mut res = Vec::new();
    let y = root_yaml_to_vec(&y, "goal")?;

    for (i, y) in y.into_iter().enumerate() {
        let mut map = try_as_map!(y, "goal", i);

        let id = try_lookup_string!(map, "id", "goal", i);
        let goal = try_lookup_string!(map, "goal", "goal", id);
        let top = try_lookup_bool!(map, "top", "goal", id);
        let pitch = try_lookup_string!(map, "pitch", "goal", id);
        let theme = try_lookup_string!(map, "theme", "goal", id);
        let tracking_link = try_lookup_string!(map, "tracking-link", "goal", id);
        let release = try_lookup_string!(map, "release", "goal", id);

        warn_extra_fields(map, "goal", &id);

        res.push(Goal {
            id: id,
            goal: goal,
            top: top,
            pitch: pitch,
            theme: theme,
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
        let theme = try_lookup_string!(map, "theme", "problem", id);

        warn_extra_fields(map, "problem", &id);

        res.push(Problem {
            id: id,
            pitch: pitch,
            theme: theme,
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
