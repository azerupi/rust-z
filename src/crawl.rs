
use std::collections::{HashMap, HashSet, VecDeque};
use std::iter;
use {Battleplan, load_plan};
use errors::*;
use url::Url;
use gh::models::IssueFromJson;
use regex::Regex;

#[derive(Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum UrlFact {
    CrawlError(String),
    GitHubIssue(IssueFromJson),
    GitHubPullRequest,
}

impl UrlFact {
    fn short(&self) -> String {
        match *self {
            UrlFact::CrawlError(ref e) => format!("crawl error: {}", e),
            UrlFact::GitHubIssue(_) => format!("is a GitHub issue"),
            UrlFact::GitHubPullRequest => format!("is a GitHub pull request"),
        }
    }
}

pub type UrlFacts = HashMap<Url, HashSet<UrlFact>>;

pub trait FactSetExt {
    fn gh_issue(&self) -> Option<&IssueFromJson>;
}

impl FactSetExt for HashSet<UrlFact> {
    fn gh_issue(&self) -> Option<&IssueFromJson> {
        for fact in self {
            match *fact {
                UrlFact::GitHubIssue(ref i) => return Some(i),
                _ => ()
            }
        }

        None
    }
}

pub fn crawl() -> Result<()> {
    let plan = load_plan()?;
    plan.validate()?;

    let urls = initial_urls_from_plan(&plan);

    let urls_with_distances = urls
        .into_iter()
        .zip(iter::repeat(0))
        .collect::<Vec<_>>();

    let mut urls = VecDeque::from(urls_with_distances);

    let mut facts: UrlFacts = HashMap::new();

    while let Some(url) = urls.pop_front() {
        if url.1 > MAX_DISTANCE { continue }
        match learn_about_url(&url, &mut urls, &mut facts) {
            Ok(_) => (),
            Err(e) => {
                add_fact(&mut facts, &url.0, UrlFact::CrawlError(format!("{}", e)));
            }
        }
    }

    write_url_facts(&facts)?;

    Ok(())
}

fn write_url_facts(facts: &UrlFacts) -> Result<()> {
    super::write_yaml("crawl", &facts)
}

pub fn load_url_facts() -> Result<UrlFacts> {
    super::load_yaml("crawl")
}

fn initial_urls_from_plan(plan: &Battleplan) -> Vec<Url> {
    let mut urls = Vec::new();
    for goal in &plan.goals {
        match Url::parse(&goal.tracking_link) {
            Ok(url) => urls.push(url),
            Err(_) => (/* bogus link */),
        }
    }

    urls
}

const MAX_DISTANCE: u32 = 5;

type Distance = u32;

fn add_fact(facts: &mut HashMap<Url, HashSet<UrlFact>>,
            url: &Url,
            fact: UrlFact) {
    info!("learned about {}: {}", url, fact.short());

    if facts.get(&url).is_none() {
        facts.insert(url.clone(), HashSet::new());
    }

    let url_facts = facts.get_mut(url).expect("");
    url_facts.insert(fact);
}

fn learn_about_url(url_d: &(Url, Distance),
                   urls: &mut VecDeque<(Url, Distance)>,
                   facts: &mut HashMap<Url, HashSet<UrlFact>>) -> Result<()> {
    info!("learning about {}", url_d.0);

    let url = &url_d.0;

    if url.as_str().starts_with("https://github.com") {
        let (new_urls, new_facts) = learn_about_github_url(url)?;

        for new_url in new_urls {
            urls.push_back((new_url, url_d.1 + 1));
        }

        for (new_url, new_fact) in new_facts {
            add_fact(facts, &new_url, new_fact);
        }
    } else {
        error!("URL not understood: {}", url);
    }

    Ok(())
}

fn learn_about_github_url(url: &Url) -> Result<(Vec<Url>, Vec<(Url, UrlFact)>)> {
    if url.as_str().contains("/issues/") {
        learn_about_github_issue(url)
    } else {
        error!("GitHub URL not understood: {}", url);
        Ok((Vec::new(), Vec::new()))
    }
}

fn learn_about_github_issue(url: &Url) -> Result<(Vec<Url>, Vec<(Url, UrlFact)>)> {
        
    use gh::client::Client;

    let mut new_urls = Vec::new();
    let mut new_facts = Vec::new();

    let (org, repo, number) = parse_gh_issue(url)?;

    let client = Client::new();

    let issue = client.fetch_issue(&org, &repo, &number)?;

    new_facts.push((url.clone(), UrlFact::GitHubIssue(issue.clone())));

    let (more_urls, more_facts) = learn_about_rfcs_from_issue(&issue)?;
    new_urls.extend(more_urls);
    new_facts.extend(more_facts);

    Ok((new_urls, new_facts))
}

fn learn_about_rfcs_from_issue(issue: &IssueFromJson)
                               -> Result<(Vec<Url>, Vec<(Url, UrlFact)>)> {
    let mut new_urls = Vec::new();

    if let Some(ref body) = issue.body {
        // Match "rust-lang/rfcs#more-than-one-digit"
        let rfc_ref_re = Regex::new(r"rust-lang/rfcs#(\d{1,})").expect("");

        let mut rfc_numbers = vec!();
        
        for line in body.lines() {
            if let Some(cap) = rfc_ref_re.captures(line) {
                rfc_numbers.push(cap.at(1).expect(""));
            }
        }

        for rfc_number in rfc_numbers {
            let rfc_url = Url::parse(&format!("https://github.com/rust-lang/rfcs/pulls/{}", rfc_number)).expect("");
            new_urls.push(rfc_url);
        }
    }

    Ok((new_urls, Vec::new()))
}


fn parse_gh_issue(url: &Url) -> Result<(String, String, String)> {
    // Parse "/$org/$repo/issues/$number" from URL
    let re = Regex::new("/(.*)/(.*)/issues/(.*)").expect("");
    let path = url.path();

    if let Some(cap) = re.captures(path) {
        let org = cap.at(1).expect("");
        let repo = cap.at(2).expect("");
        let number = cap.at(3).expect("");
        Ok((org.into(), repo.into(), number.into()))
    } else {
        Err(format!("can't parse GitHub issue url {}", url).into())
    }
}
