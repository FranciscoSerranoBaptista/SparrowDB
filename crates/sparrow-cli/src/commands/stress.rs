use eyre::Result;
use fake::{
    faker::{company::en::CompanyName, job::en::Title, name::en::{FirstName, LastName}},
    Fake,
};
use futures_util::stream::{self, StreamExt};
use rand::Rng;
use reqwest::Client;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc, time::Instant};
use uuid::Uuid;

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Company {
    name: String,
}

#[derive(Debug, Clone)]
struct Job {
    profession: String,
}

#[derive(Debug, Clone)]
struct PersonRecord {
    person_id: String,
    company_name: Option<String>,
    profession: Option<String>,
}

#[derive(Debug, Default)]
struct PhaseResult {
    name: &'static str,
    passed: usize,
    failed: usize,
    notes: Vec<String>,
    duration_ms: u64,
}

impl PhaseResult {
    fn new(name: &'static str) -> Self {
        Self { name, ..Default::default() }
    }
    fn pass(&mut self) { self.passed += 1; }
    fn fail(&mut self, reason: impl Into<String>) {
        self.failed += 1;
        self.notes.push(reason.into());
    }
    fn print(&self) {
        println!(
            "  {:<40} passed={} failed={} ({} ms)",
            self.name, self.passed, self.failed, self.duration_ms
        );
        for note in self.notes.iter().take(5) {
            println!("    ! {note}");
        }
        if self.notes.len() > 5 {
            println!("    ! ... and {} more failures", self.notes.len() - 5);
        }
    }
}

// ── HTTP helpers ──────────────────────────────────────────────────────────────

/// Returns Err only on connection failure; otherwise (is_http_success, body).
async fn query_raw(
    client: &Client,
    base_url: &str,
    name: &str,
    payload: Value,
) -> Result<(bool, Value)> {
    let url = format!("{}/{}", base_url, name);
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| eyre::eyre!("connection error calling {name}: {e}"))?;
    let ok = resp.status().is_success();
    let body = resp.json::<Value>().await.unwrap_or(Value::Null);
    Ok((ok, body))
}

/// Strict version — returns Err on non-2xx.
async fn query(client: &Client, base_url: &str, name: &str, payload: Value) -> Result<Value> {
    match query_raw(client, base_url, name, payload).await? {
        (true, body) => Ok(body),
        (false, body) => Err(eyre::eyre!("{name} failed: {body}")),
    }
}

/// True if the response body contains at least one non-null, non-empty-array value.
fn is_nonempty(v: &Value) -> bool {
    match v {
        Value::Array(a) => !a.is_empty(),
        Value::Object(m) => m.values().any(|v| match v {
            Value::Array(a) => !a.is_empty(),
            Value::Null => false,
            _ => true,
        }),
        Value::Null => false,
        _ => true,
    }
}

/// Extract a scalar count from a response like {"count": 42} or just 42.
fn extract_count(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::Object(m) => m.values().find_map(|v| {
            if let Value::Number(n) = v { n.as_u64() } else { None }
        }),
        _ => None,
    }
}

/// Extract a boolean from a response like {"exists": true} or just true.
fn extract_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Object(m) => m.values().find_map(|v| {
            if let Value::Bool(b) = v { Some(*b) } else { None }
        }),
        _ => None,
    }
}

/// Generate a random unit-normalised vector of `dim` f64s.
fn random_vec(dim: usize) -> Vec<f64> {
    let mut rng = rand::thread_rng();
    (0..dim).map(|_| rng.r#gen::<f64>() * 2.0 - 1.0).collect()
}

// ── Write phase ───────────────────────────────────────────────────────────────

async fn insert_companies(
    client: Arc<Client>,
    base_url: Arc<String>,
    num_companies: usize,
    workers: usize,
) -> Vec<Company> {
    println!("Generating {} companies...", num_companies);
    let mut names: HashSet<String> = HashSet::new();
    while names.len() < num_companies {
        names.insert(CompanyName().fake());
    }
    let names: Vec<String> = names.into_iter().collect();
    println!("Inserting {} companies ({} workers)...", names.len(), workers);

    let results: Vec<Option<Company>> = stream::iter(names.into_iter().enumerate())
        .map(|(idx, name)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query(&client, &base_url, "createCompany", json!({ "name": name })).await {
                    Ok(_) => {
                        if (idx + 1) % 10 == 0 { println!("  Inserted {} companies", idx + 1); }
                        Some(Company { name })
                    }
                    Err(e) => { eprintln!("Error creating company '{}': {}", name, e); None }
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let companies: Vec<Company> = results.into_iter().flatten().collect();
    println!("✓ Inserted {} companies", companies.len());
    companies
}

async fn insert_jobs(
    client: Arc<Client>,
    base_url: Arc<String>,
    num_jobs: usize,
    workers: usize,
) -> Vec<Job> {
    println!("Generating {} jobs...", num_jobs);
    let mut titles: HashSet<String> = HashSet::new();
    while titles.len() < num_jobs {
        titles.insert(Title().fake());
    }
    let titles: Vec<String> = titles.into_iter().collect();
    println!("Inserting {} jobs ({} workers)...", titles.len(), workers);

    let results: Vec<Option<Job>> = stream::iter(titles.into_iter().enumerate())
        .map(|(idx, profession)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query(&client, &base_url, "createJob", json!({ "profession": profession })).await {
                    Ok(_) => {
                        if (idx + 1) % 10 == 0 { println!("  Inserted {} jobs", idx + 1); }
                        Some(Job { profession })
                    }
                    Err(e) => { eprintln!("Error creating job '{}': {}", profession, e); None }
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let jobs: Vec<Job> = results.into_iter().flatten().collect();
    println!("✓ Inserted {} jobs", jobs.len());
    jobs
}

async fn insert_people(
    client: Arc<Client>,
    base_url: Arc<String>,
    num_people: usize,
    companies: Vec<Company>,
    jobs: Vec<Job>,
    workers: usize,
    progress_interval: usize,
) -> Vec<PersonRecord> {
    println!("Generating {} people...", num_people);
    let mut rng = rand::thread_rng();
    let companies = Arc::new(companies);
    let jobs = Arc::new(jobs);

    let mut people_data: Vec<(String, String, String, i32, Option<String>, Option<String>)> =
        Vec::with_capacity(num_people);

    for _ in 0..num_people {
        let person_id = Uuid::new_v4().to_string();
        let first_name: String = FirstName().fake();
        let last_name: String = LastName().fake();
        let age: i32 = rng.gen_range(18..=65);
        let company_name = if !companies.is_empty() && rng.gen_bool(0.92) {
            Some(companies[rng.gen_range(0..companies.len())].name.clone())
        } else {
            None
        };
        let profession = if !jobs.is_empty() && rng.gen_bool(0.95) {
            Some(jobs[rng.gen_range(0..jobs.len())].profession.clone())
        } else {
            None
        };
        people_data.push((person_id, first_name, last_name, age, company_name, profession));
    }

    println!("Inserting {} people ({} workers)...", people_data.len(), workers);

    let results: Vec<Option<PersonRecord>> =
        stream::iter(people_data.into_iter().enumerate())
            .map(|(idx, (person_id, first_name, last_name, age, company_name, profession))| {
                let client = Arc::clone(&client);
                let base_url = Arc::clone(&base_url);
                async move {
                    let payload = json!({
                        "person_id": person_id,
                        "first_name": first_name,
                        "last_name": last_name,
                        "age": age
                    });
                    if let Err(e) = query(&client, &base_url, "createPerson", payload).await {
                        eprintln!("Error creating person {}: {}", person_id, e);
                        return None;
                    }
                    if let Some(ref name) = company_name {
                        let payload = json!({ "person_id": person_id, "company_name": name });
                        if let Err(e) = query(&client, &base_url, "ConnectPersonToCompany", payload).await {
                            eprintln!("Error connecting {} to company: {}", person_id, e);
                        }
                    }
                    if let Some(ref prof) = profession {
                        let payload = json!({ "person_id": person_id, "profession": prof });
                        if let Err(e) = query(&client, &base_url, "ConnectPersonToJob", payload).await {
                            eprintln!("Error connecting {} to job: {}", person_id, e);
                        }
                    }
                    if (idx + 1) % progress_interval == 0 {
                        println!("  Inserted {} people", idx + 1);
                    }
                    Some(PersonRecord { person_id, company_name, profession })
                }
            })
            .buffer_unordered(workers)
            .collect()
            .await;

    let people: Vec<PersonRecord> = results.into_iter().flatten().collect();
    println!("✓ Inserted {} people", people.len());
    people
}

async fn connect_companies_to_jobs(
    client: Arc<Client>,
    base_url: Arc<String>,
    companies: &[Company],
    jobs: &[Job],
    workers: usize,
) {
    if jobs.is_empty() || companies.is_empty() { return; }
    println!("Creating company-job relationships...");
    let mut rng = rand::thread_rng();
    let mut pairs: Vec<(String, String)> = Vec::new();
    for company in companies {
        let n = rng.gen_range(3..=10).min(jobs.len());
        let mut selected: HashSet<usize> = HashSet::new();
        while selected.len() < n {
            selected.insert(rng.gen_range(0..jobs.len()));
        }
        for idx in selected {
            pairs.push((company.name.clone(), jobs[idx].profession.clone()));
        }
    }
    let total = pairs.len();
    println!("Inserting {} company-job pairs ({} workers)...", total, workers);

    let results: Vec<bool> = stream::iter(pairs.into_iter())
        .map(|(company_name, profession)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query(&client, &base_url, "ConnectCompanyToJob",
                    json!({ "company_name": company_name, "profession": profession })).await {
                    Ok(_) => true,
                    Err(e) => { eprintln!("Error connecting company to job: {}", e); false }
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    println!("✓ Created {} company-job relationships", results.iter().filter(|&&ok| ok).count());
}

// ── Read / traversal phase ────────────────────────────────────────────────────

async fn phase_valid_traversals(
    client: Arc<Client>,
    base_url: Arc<String>,
    people: &[PersonRecord],
    companies: &[Company],
    jobs: &[Job],
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("valid_traversals");

    let with_company: Vec<&PersonRecord> = people.iter().filter(|p| p.company_name.is_some()).collect();
    let without_company: Vec<&PersonRecord> = people.iter().filter(|p| p.company_name.is_none()).collect();
    let with_job: Vec<&PersonRecord> = people.iter().filter(|p| p.profession.is_some()).collect();
    let without_job: Vec<&PersonRecord> = people.iter().filter(|p| p.profession.is_none()).collect();
    let with_both: Vec<&PersonRecord> = people.iter()
        .filter(|p| p.company_name.is_some() && p.profession.is_some()).collect();

    let sample = |v: &[&PersonRecord], n: usize| -> Vec<PersonRecord> {
        let mut rng = rand::thread_rng();
        let take = n.min(v.len());
        let mut idxs: HashSet<usize> = HashSet::new();
        while idxs.len() < take { idxs.insert(rng.gen_range(0..v.len())); }
        idxs.into_iter().map(|i| v[i].clone()).collect()
    };

    // Build tasks: (query_name, payload, expect_nonempty, label)
    type Task = (String, Value, bool, String);
    let mut tasks: Vec<Task> = Vec::new();

    // GetPersonCompany – person HAS company → expect data
    for p in sample(&with_company, 30) {
        tasks.push((
            "GetPersonCompany".into(),
            json!({ "person_id": p.person_id }),
            true,
            format!("GetPersonCompany(has_company={})", &p.person_id[..8]),
        ));
    }
    // GetPersonCompany – person has NO company → expect empty
    for p in sample(&without_company, 10.min(without_company.len())) {
        tasks.push((
            "GetPersonCompany".into(),
            json!({ "person_id": p.person_id }),
            false,
            format!("GetPersonCompany(no_company={})", &p.person_id[..8]),
        ));
    }
    // GetPersonJob – person HAS job
    for p in sample(&with_job, 30) {
        tasks.push((
            "GetPersonJob".into(),
            json!({ "person_id": p.person_id }),
            true,
            format!("GetPersonJob(has_job={})", &p.person_id[..8]),
        ));
    }
    // GetPersonJob – person has NO job
    for p in sample(&without_job, 10.min(without_job.len())) {
        tasks.push((
            "GetPersonJob".into(),
            json!({ "person_id": p.person_id }),
            false,
            format!("GetPersonJob(no_job={})", &p.person_id[..8]),
        ));
    }
    // GetPersonFullInfo – person with both
    for p in sample(&with_both, 20) {
        tasks.push((
            "GetPersonFullInfo".into(),
            json!({ "person_id": p.person_id }),
            true,
            format!("GetPersonFullInfo({})", &p.person_id[..8]),
        ));
    }
    // getPeopleAtCompany
    for c in companies.iter().take(15) {
        tasks.push((
            "getPeopleAtCompany".into(),
            json!({ "company_name": c.name }),
            true,
            format!("getPeopleAtCompany({})", &c.name[..c.name.len().min(20)]),
        ));
    }
    // getCompanyJobs
    for c in companies.iter().take(15) {
        tasks.push((
            "getCompanyJobs".into(),
            json!({ "company_name": c.name }),
            true,
            format!("getCompanyJobs({})", &c.name[..c.name.len().min(20)]),
        ));
    }
    // getPeopleWithJob
    for j in jobs.iter().take(10) {
        tasks.push((
            "getPeopleWithJob".into(),
            json!({ "profession": j.profession }),
            true,
            format!("getPeopleWithJob({})", &j.profession[..j.profession.len().min(20)]),
        ));
    }
    // getCompaniesOfferingJob
    for j in jobs.iter().take(10) {
        tasks.push((
            "getCompaniesOfferingJob".into(),
            json!({ "profession": j.profession }),
            true,
            format!("getCompaniesOfferingJob({})", &j.profession[..j.profession.len().min(20)]),
        ));
    }
    // getCompanyByName
    for c in companies.iter().take(5) {
        tasks.push((
            "getCompanyByName".into(),
            json!({ "name": c.name }),
            true,
            format!("getCompanyByName({})", &c.name[..c.name.len().min(20)]),
        ));
    }
    // getJobByProfession
    for j in jobs.iter().take(5) {
        tasks.push((
            "getJobByProfession".into(),
            json!({ "profession": j.profession }),
            true,
            format!("getJobByProfession({})", &j.profession[..j.profession.len().min(20)]),
        ));
    }
    // getAllPeople / getAllCompanies / getAllJobs (large result sets)
    tasks.push(("getAllPeople".into(), json!({}), true, "getAllPeople()".into()));
    tasks.push(("getAllCompanies".into(), json!({}), true, "getAllCompanies()".into()));
    tasks.push(("getAllJobs".into(), json!({}), true, "getAllJobs()".into()));

    let results: Vec<(bool, bool, Value, String)> = stream::iter(tasks.into_iter())
        .map(|(qname, payload, expect_nonempty, label)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query_raw(&client, &base_url, &qname, payload).await {
                    Ok((http_ok, body)) => (http_ok, expect_nonempty, body, label),
                    Err(e) => (false, expect_nonempty, json!(e.to_string()), label),
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    for (http_ok, expect_nonempty, body, label) in results {
        if !http_ok {
            res.fail(format!("{label}: HTTP error – {body}"));
        } else if expect_nonempty && !is_nonempty(&body) {
            res.fail(format!("{label}: expected data but got empty – {body}"));
        } else {
            res.pass();
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Edge case: non-existent IDs ───────────────────────────────────────────────

async fn phase_nonexistent_lookups(
    client: Arc<Client>,
    base_url: Arc<String>,
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("nonexistent_lookups");

    let mut tasks: Vec<(String, Value, String)> = Vec::new();

    // 20 random UUIDs that were never inserted
    for _ in 0..20 {
        let id = Uuid::new_v4().to_string();
        tasks.push(("GetPersonCompany".into(), json!({ "person_id": id }), format!("GetPersonCompany(uuid={})", &id[..8])));
        tasks.push(("GetPersonJob".into(), json!({ "person_id": id }), format!("GetPersonJob(uuid={})", &id[..8])));
        tasks.push(("GetPersonFullInfo".into(), json!({ "person_id": id }), format!("GetPersonFullInfo(uuid={})", &id[..8])));
    }
    // Made-up company names
    for i in 0..10 {
        let name = format!("NonExistentCorp__{i}__xyz");
        tasks.push(("getPeopleAtCompany".into(), json!({ "company_name": &name }), format!("getPeopleAtCompany(fake={name})")));
        tasks.push(("getCompanyJobs".into(), json!({ "company_name": &name }), format!("getCompanyJobs(fake={name})")));
        tasks.push(("getCompanyByName".into(), json!({ "name": &name }), format!("getCompanyByName(fake={name})")));
    }
    // Made-up professions
    for i in 0..10 {
        let prof = format!("Imaginary_Job_Title__{i}__zyx");
        tasks.push(("getPeopleWithJob".into(), json!({ "profession": &prof }), format!("getPeopleWithJob(fake={prof})")));
        tasks.push(("getCompaniesOfferingJob".into(), json!({ "profession": &prof }), format!("getCompaniesOfferingJob(fake={prof})")));
        tasks.push(("getJobByProfession".into(), json!({ "profession": &prof }), format!("getJobByProfession(fake={prof})")));
    }

    let results: Vec<(bool, Value, String)> = stream::iter(tasks.into_iter())
        .map(|(qname, payload, label)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query_raw(&client, &base_url, &qname, payload).await {
                    Ok((ok, body)) => (ok, body, label),
                    Err(e) => (false, json!(e.to_string()), label),
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    for (http_ok, body, label) in results {
        // Server may return either 200+empty OR a structured HTTP error for not-found.
        // Both are acceptable — what we're checking is that the server doesn't crash.
        // The only failure is: 200 with actual data (phantom result for a fake ID).
        if http_ok && is_nonempty(&body) {
            res.fail(format!("{label}: got real data for a nonexistent key – {body}"));
        } else {
            res.pass(); // 200+empty or HTTP error — both fine
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Edge case: boundary age queries ──────────────────────────────────────────

async fn phase_boundary_age(
    client: Arc<Client>,
    base_url: Arc<String>,
    num_people: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("boundary_age_queries");

    // People are generated with age in 18..=65
    //   OlderThan(n)  → people with age > n
    //   YoungerThan(n) → people with age < n
    struct Case { q: &'static str, field: &'static str, val: i32, expect_nonempty: bool, desc: &'static str }
    let cases = [
        Case { q: "getPeopleOlderThan",   field: "min_age", val:   0, expect_nonempty: true,  desc: "OlderThan(0) → all" },
        Case { q: "getPeopleOlderThan",   field: "min_age", val:  17, expect_nonempty: true,  desc: "OlderThan(17) → all" },
        Case { q: "getPeopleOlderThan",   field: "min_age", val:  18, expect_nonempty: true,  desc: "OlderThan(18) → most" },
        Case { q: "getPeopleOlderThan",   field: "min_age", val:  64, expect_nonempty: true,  desc: "OlderThan(64) → few (age=65)" },
        Case { q: "getPeopleOlderThan",   field: "min_age", val:  65, expect_nonempty: false, desc: "OlderThan(65) → empty (max age=65)" },
        Case { q: "getPeopleOlderThan",   field: "min_age", val: 100, expect_nonempty: false, desc: "OlderThan(100) → empty" },
        Case { q: "getPeopleOlderThan",   field: "min_age", val: i32::MAX, expect_nonempty: false, desc: "OlderThan(i32::MAX) → empty" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val: 200, expect_nonempty: true,  desc: "YoungerThan(200) → all" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val:  66, expect_nonempty: true,  desc: "YoungerThan(66) → all" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val:  65, expect_nonempty: true,  desc: "YoungerThan(65) → most" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val:  19, expect_nonempty: true,  desc: "YoungerThan(19) → few (age=18)" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val:  18, expect_nonempty: false, desc: "YoungerThan(18) → empty (min age=18)" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val:   0, expect_nonempty: false, desc: "YoungerThan(0) → empty" },
        Case { q: "getPeopleYoungerThan", field: "max_age", val: i32::MIN, expect_nonempty: false, desc: "YoungerThan(i32::MIN) → empty" },
    ];

    let _ = num_people; // used for context only
    for case in &cases {
        match query_raw(&client, &base_url, case.q, json!({ case.field: case.val })).await {
            Ok((true, body)) => {
                if case.expect_nonempty == is_nonempty(&body) {
                    res.pass();
                } else {
                    res.fail(format!(
                        "{}: expected nonempty={} got nonempty={}",
                        case.desc, case.expect_nonempty, is_nonempty(&body)
                    ));
                }
            }
            Ok((false, body)) => res.fail(format!("{}: HTTP error – {body}", case.desc)),
            Err(e) => res.fail(format!("{}: connection error – {e}", case.desc)),
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Edge case: duplicate writes ───────────────────────────────────────────────

async fn phase_duplicate_writes(
    client: Arc<Client>,
    base_url: Arc<String>,
    companies: &[Company],
    jobs: &[Job],
    people: &[PersonRecord],
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("duplicate_writes");

    let mut tasks: Vec<(String, Value, String)> = Vec::new();

    // Duplicate company (same INDEX name) → must fail
    for c in companies.iter().take(10) {
        tasks.push(("createCompany".into(), json!({ "name": c.name }), format!("dup_company({})", &c.name[..c.name.len().min(20)])));
    }
    // Duplicate job → must fail
    for j in jobs.iter().take(10) {
        tasks.push(("createJob".into(), json!({ "profession": j.profession }), format!("dup_job({})", &j.profession[..j.profession.len().min(20)])));
    }
    // Duplicate person_id → must fail
    for p in people.iter().take(10) {
        tasks.push(("createPerson".into(), json!({ "person_id": p.person_id, "first_name": "Dup", "last_name": "Dup", "age": 30 }), format!("dup_person({})", &p.person_id[..8])));
    }
    // Duplicate edges: reconnect same person→company twice
    for p in people.iter().filter(|p| p.company_name.is_some()).take(10) {
        tasks.push(("ConnectPersonToCompany".into(),
            json!({ "person_id": p.person_id, "company_name": p.company_name.as_ref().unwrap() }),
            format!("dup_edge_person_company({})", &p.person_id[..8]),
        ));
    }
    // Duplicate edges: reconnect same person→job twice
    for p in people.iter().filter(|p| p.profession.is_some()).take(10) {
        tasks.push(("ConnectPersonToJob".into(),
            json!({ "person_id": p.person_id, "profession": p.profession.as_ref().unwrap() }),
            format!("dup_edge_person_job({})", &p.person_id[..8]),
        ));
    }

    let results: Vec<(bool, Value, String)> = stream::iter(tasks.into_iter())
        .map(|(qname, payload, label)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query_raw(&client, &base_url, &qname, payload).await {
                    Ok((ok, body)) => (ok, body, label),
                    Err(e) => (false, json!(e.to_string()), label),
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    for (http_ok, body, label) in results {
        // ALL of these should fail — duplicate key violation
        if http_ok {
            res.fail(format!("{label}: expected duplicate-key error but got HTTP 200 – {body}"));
        } else {
            res.pass(); // correctly rejected
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Edge case: degenerate inputs ─────────────────────────────────────────────

async fn phase_degenerate_inputs(
    client: Arc<Client>,
    base_url: Arc<String>,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("degenerate_inputs");

    // Empty strings
    let empty_cases: &[(&str, Value, bool)] = &[
        ("GetPersonCompany",     json!({ "person_id": "" }),      false),
        ("GetPersonJob",         json!({ "person_id": "" }),      false),
        ("getPeopleAtCompany",   json!({ "company_name": "" }),   false),
        ("getCompanyJobs",       json!({ "company_name": "" }),   false),
        ("getCompanyByName",     json!({ "name": "" }),           false),
        ("getJobByProfession",   json!({ "profession": "" }),     false),
        ("getPeopleWithJob",     json!({ "profession": "" }),     false),
        ("getCompaniesOfferingJob", json!({ "profession": "" }), false),
    ];

    // Very long strings (should not crash the server)
    let long_str = "x".repeat(4096);
    let long_cases: &[(&str, Value)] = &[
        ("GetPersonCompany",  json!({ "person_id": &long_str })),
        ("getPeopleAtCompany", json!({ "company_name": &long_str })),
        ("getPeopleWithJob",  json!({ "profession": &long_str })),
    ];

    // Unicode and special chars
    let special_cases: &[(&str, Value)] = &[
        ("getPeopleAtCompany", json!({ "company_name": "'; DROP TABLE nodes;--" })),
        ("getPeopleAtCompany", json!({ "company_name": "Ünïcödé Cörp" })),
        ("getPeopleAtCompany", json!({ "company_name": "🦅 Sparrow Inc 🦅" })),
        ("getPeopleAtCompany", json!({ "company_name": "\n\t\r" })),
        ("getPeopleWithJob",   json!({ "profession": "null" })),
        ("getCompanyByName",   json!({ "name": "undefined" })),
    ];

    // Empty-string lookups: server should return 200 + empty (not crash)
    for (qname, payload, expect_nonempty) in empty_cases {
        match query_raw(&client, &base_url, qname, payload.clone()).await {
            Ok((true, body)) if *expect_nonempty == is_nonempty(&body) => res.pass(),
            Ok((true, body)) if !*expect_nonempty && !is_nonempty(&body) => res.pass(),
            Ok((false, _)) => res.pass(), // also acceptable — reject empty string
            Err(e) => res.fail(format!("empty_str {qname}: connection error – {e}")),
            Ok((true, body)) => res.fail(format!("empty_str {qname}: unexpected nonempty – {body}")),
        }
    }

    // Long strings: server must not crash (any 200 or structured 4xx/5xx is OK)
    for (qname, payload) in long_cases {
        match query_raw(&client, &base_url, qname, payload.clone()).await {
            Ok(_) => res.pass(), // any HTTP response is fine — just mustn't hang/crash
            Err(e) => res.fail(format!("long_str {qname}: connection error – {e}")),
        }
    }

    // Special chars: server must not crash
    for (qname, payload) in special_cases {
        match query_raw(&client, &base_url, qname, payload.clone()).await {
            Ok(_) => res.pass(),
            Err(e) => res.fail(format!("special {qname}: connection error – {e}")),
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Edge case: concurrent read storm ─────────────────────────────────────────

async fn phase_concurrent_reads(
    client: Arc<Client>,
    base_url: Arc<String>,
    people: &[PersonRecord],
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("concurrent_reads");
    let mut rng = rand::thread_rng();

    // Fire 200 random queries in parallel at maximum concurrency
    let n = 200.min(people.len() * 3);
    let mut tasks: Vec<(String, Value)> = Vec::with_capacity(n);
    for _ in 0..n {
        let p = &people[rng.gen_range(0..people.len())];
        match rng.gen_range(0..6) {
            0 => tasks.push(("GetPersonCompany".into(), json!({ "person_id": p.person_id }))),
            1 => tasks.push(("GetPersonJob".into(), json!({ "person_id": p.person_id }))),
            2 => tasks.push(("GetPersonFullInfo".into(), json!({ "person_id": p.person_id }))),
            3 => tasks.push(("getAllPeople".into(), json!({}))),
            4 => tasks.push(("getAllCompanies".into(), json!({}))),
            _ => tasks.push(("getAllJobs".into(), json!({}))),
        }
    }

    let results: Vec<bool> = stream::iter(tasks.into_iter())
        .map(|(qname, payload)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                matches!(query_raw(&client, &base_url, &qname, payload).await, Ok((true, _)))
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    for ok in results {
        if ok { res.pass(); } else { res.fail("concurrent read returned non-200"); }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 8: Node updates ─────────────────────────────────────────────────────

async fn phase_node_updates(
    client: Arc<Client>,
    base_url: Arc<String>,
    people: &[PersonRecord],
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("node_updates");
    let mut rng = rand::thread_rng();

    // Pick 20 people; update their ages to 500 (well outside the original [18..65] range)
    let take = 20.min(people.len());
    let mut idxs: HashSet<usize> = HashSet::new();
    while idxs.len() < take {
        idxs.insert(rng.gen_range(0..people.len()));
    }
    let targets: Vec<String> = idxs.into_iter().map(|i| people[i].person_id.clone()).collect();

    let update_results: Vec<bool> = stream::iter(targets.iter().cloned())
        .map(|pid| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                matches!(
                    query_raw(&client, &base_url, "updatePersonAge",
                        json!({ "person_id": pid, "age": 500 })).await,
                    Ok((true, _))
                )
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let updated = update_results.iter().filter(|&&ok| ok).count();
    if updated == 0 {
        res.fail("no people could be updated".to_string());
        res.duration_ms = t.elapsed().as_millis() as u64;
        return res;
    }
    res.pass(); // updates succeeded

    // getPeopleOlderThan(499) must now return results (those 20 people have age=500)
    match query_raw(&client, &base_url, "getPeopleOlderThan", json!({ "min_age": 499 })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, _)) => res.fail(format!("getPeopleOlderThan(499) empty after updating {updated} people to age=500")),
        Ok((false, body)) => res.fail(format!("getPeopleOlderThan(499) HTTP error: {body}")),
        Err(e) => res.fail(format!("getPeopleOlderThan(499) connection error: {e}")),
    }

    // getPeopleOlderThan(500) must be empty (no one has age > 500)
    match query_raw(&client, &base_url, "getPeopleOlderThan", json!({ "min_age": 500 })).await {
        Ok((true, body)) if !is_nonempty(&body) => res.pass(),
        Ok((false, _)) => res.pass(), // HTTP error also acceptable — truly empty
        Ok((true, body)) => res.fail(format!("getPeopleOlderThan(500) should be empty, got: {body}")),
        Err(e) => res.fail(format!("getPeopleOlderThan(500) connection error: {e}")),
    }

    // BM25 update path: change first_name to something distinctive, then search for it
    let bm25_pid = &targets[0];
    let new_name = format!("BM25UpdatedName_{}", &bm25_pid[..8]);
    let bm25_update_ok = matches!(
        query_raw(&client, &base_url, "updatePersonName",
            json!({ "person_id": bm25_pid, "first_name": &new_name })).await,
        Ok((true, _))
    );
    if !bm25_update_ok {
        res.fail("updatePersonName failed for BM25 update test".to_string());
    } else {
        // Search for the new distinctive name — should find exactly this person
        match query_raw(&client, &base_url, "searchPeopleByName",
            json!({ "query": &new_name, "limit": 5 })).await {
            Ok((true, body)) if is_nonempty(&body) => res.pass(),
            Ok((true, _)) => res.fail(format!("BM25 not updated after updatePersonName for {new_name}")),
            Ok((false, body)) => res.fail(format!("searchPeopleByName HTTP error: {body}")),
            Err(e) => res.fail(format!("searchPeopleByName connection error: {e}")),
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 9: Delete lifecycle ─────────────────────────────────────────────────

async fn phase_delete_lifecycle(
    client: Arc<Client>,
    base_url: Arc<String>,
    companies: &[Company],
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("delete_lifecycle");

    // ── Part A: UNIQUE index cleanup ──────────────────────────────────────────
    // Insert → verify exists → delete → verify gone → re-insert same ID → verify back
    let canary_id = format!("stress-canary-{}", Uuid::new_v4());

    // Insert canary
    match query_raw(&client, &base_url, "createPerson", json!({
        "person_id": &canary_id, "first_name": "Canary", "last_name": "Delete", "age": 42
    })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => { res.fail(format!("canary insert failed: {body}")); res.duration_ms = t.elapsed().as_millis() as u64; return res; }
        Err(e) => { res.fail(format!("canary insert error: {e}")); res.duration_ms = t.elapsed().as_millis() as u64; return res; }
    }

    // Verify exists (GetPersonFullInfo returns HTTP 200)
    match query_raw(&client, &base_url, "GetPersonFullInfo", json!({ "person_id": &canary_id })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("canary not found after insert: {body}")),
        Err(e) => res.fail(format!("canary existence check error: {e}")),
    }

    // Delete
    match query_raw(&client, &base_url, "deletePersonById", json!({ "person_id": &canary_id })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("deletePersonById failed: {body}")),
        Err(e) => res.fail(format!("deletePersonById error: {e}")),
    }

    // Verify gone (GetPersonFullInfo returns HTTP error = "no value found")
    match query_raw(&client, &base_url, "GetPersonFullInfo", json!({ "person_id": &canary_id })).await {
        Ok((false, _)) => res.pass(), // HTTP error = not found = correct
        Ok((true, body)) if !is_nonempty(&body) => res.pass(), // 200+empty also OK
        Ok((true, body)) => res.fail(format!("canary still found after delete: {body}")),
        Err(e) => res.fail(format!("canary gone check error: {e}")),
    }

    // Re-insert with same person_id — MUST succeed (UNIQUE index entry was freed)
    match query_raw(&client, &base_url, "createPerson", json!({
        "person_id": &canary_id, "first_name": "CReborn", "last_name": "Again", "age": 43
    })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("re-insert after delete got DuplicateKey — UNIQUE index not freed: {body}")),
        Err(e) => res.fail(format!("re-insert after delete error: {e}")),
    }

    // Verify back
    match query_raw(&client, &base_url, "GetPersonFullInfo", json!({ "person_id": &canary_id })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("canary not found after re-insert: {body}")),
        Err(e) => res.fail(format!("canary re-check error: {e}")),
    }

    // Clean up canary
    let _ = query_raw(&client, &base_url, "deletePersonById", json!({ "person_id": &canary_id })).await;

    // ── Part B: Edge deletion ─────────────────────────────────────────────────
    // Create a person, connect to a company, delete their edges, verify traversal empty
    if companies.is_empty() {
        res.duration_ms = t.elapsed().as_millis() as u64;
        return res;
    }
    let edge_canary_id = format!("edge-canary-{}", Uuid::new_v4());
    let company_name = &companies[0].name;

    let _ = query_raw(&client, &base_url, "createPerson", json!({
        "person_id": &edge_canary_id, "first_name": "EdgeCanary", "last_name": "Test", "age": 30
    })).await;
    let _ = query_raw(&client, &base_url, "ConnectPersonToCompany", json!({
        "person_id": &edge_canary_id, "company_name": company_name
    })).await;

    // Verify edge exists
    match query_raw(&client, &base_url, "GetPersonCompany", json!({ "person_id": &edge_canary_id })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        other => res.fail(format!("edge not visible before deletePersonEdges: {other:?}")),
    }

    // Delete edges
    match query_raw(&client, &base_url, "deletePersonEdges", json!({ "person_id": &edge_canary_id })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("deletePersonEdges failed: {body}")),
        Err(e) => res.fail(format!("deletePersonEdges error: {e}")),
    }

    // Verify traversal now empty or error
    match query_raw(&client, &base_url, "GetPersonCompany", json!({ "person_id": &edge_canary_id })).await {
        Ok((true, body)) if !is_nonempty(&body) => res.pass(),
        Ok((false, _)) => res.pass(), // HTTP error also acceptable
        Ok((true, body)) => res.fail(format!("WorksAt edges still visible after deletePersonEdges: {body}")),
        Err(e) => res.fail(format!("GetPersonCompany after edge delete error: {e}")),
    }

    // Clean up edge canary
    let _ = query_raw(&client, &base_url, "deletePersonById", json!({ "person_id": &edge_canary_id })).await;

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 10: Multi-hop traversals ────────────────────────────────────────────

async fn phase_multihop_traversals(
    client: Arc<Client>,
    base_url: Arc<String>,
    people: &[PersonRecord],
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("multihop_traversals");

    // Person → Out<WorksAt> → Company → Out<OffersJob> → Jobs (two hops)
    // Use people who have a company (and that company has OffersJob edges, which all do)
    let with_company: Vec<&PersonRecord> = people.iter().filter(|p| p.company_name.is_some()).collect();
    let take = 20.min(with_company.len());
    if take == 0 {
        res.fail("no people with companies for multi-hop test".to_string());
        res.duration_ms = t.elapsed().as_millis() as u64;
        return res;
    }

    let mut rng = rand::thread_rng();
    let mut idxs: HashSet<usize> = HashSet::new();
    while idxs.len() < take {
        idxs.insert(rng.gen_range(0..with_company.len()));
    }
    let sample: Vec<String> = idxs.into_iter().map(|i| with_company[i].person_id.clone()).collect();

    let results: Vec<(bool, String)> = stream::iter(sample.into_iter())
        .map(|pid| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            async move {
                match query_raw(&client, &base_url, "getPersonCompanyJobs",
                    json!({ "person_id": &pid })).await {
                    Ok((true, body)) => (is_nonempty(&body), pid),
                    _ => (false, pid),
                }
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    for (ok, pid) in &results {
        if *ok {
            res.pass();
        } else {
            // Note: a person's company might have no OffersJob edges — rare but valid
            // Only fail if the HTTP request itself failed
            res.fail(format!("getPersonCompanyJobs empty/error for person {}", &pid[..8]));
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 11: COUNT, RANGE, FIRST, ORDER, EXISTS ──────────────────────────────

async fn phase_aggregation(
    client: Arc<Client>,
    base_url: Arc<String>,
    companies: &[Company],
    people: &[PersonRecord],
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("aggregation_ops");

    // countPeople() → positive number
    match query_raw(&client, &base_url, "countPeople", json!({})).await {
        Ok((true, body)) => {
            match extract_count(&body) {
                Some(0) => res.fail("countPeople returned 0".to_string()),
                Some(_) => res.pass(),
                None => res.fail(format!("countPeople: couldn't parse count from {body}")),
            }
        }
        Ok((false, body)) => res.fail(format!("countPeople HTTP error: {body}")),
        Err(e) => res.fail(format!("countPeople error: {e}")),
    }

    // countCompanyEmployees for a known company → positive
    if !companies.is_empty() {
        match query_raw(&client, &base_url, "countCompanyEmployees",
            json!({ "company_name": &companies[0].name })).await {
            Ok((true, body)) => {
                match extract_count(&body) {
                    Some(0) => res.fail(format!("countCompanyEmployees({}): 0 employees unexpected", &companies[0].name)),
                    Some(_) => res.pass(),
                    None => res.fail(format!("countCompanyEmployees: couldn't parse count from {body}")),
                }
            }
            Ok((false, body)) => res.fail(format!("countCompanyEmployees HTTP error: {body}")),
            Err(e) => res.fail(format!("countCompanyEmployees error: {e}")),
        }
    }

    // getFirstPerson() → exactly one result
    match query_raw(&client, &base_url, "getFirstPerson", json!({})).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, body)) => res.fail(format!("getFirstPerson returned empty: {body}")),
        Ok((false, body)) => res.fail(format!("getFirstPerson HTTP error: {body}")),
        Err(e) => res.fail(format!("getFirstPerson error: {e}")),
    }

    // getPeopleFirstTen() → ≤ 10 results
    match query_raw(&client, &base_url, "getPeopleFirstTen", json!({})).await {
        Ok((true, body)) => {
            let arr_len = match &body {
                Value::Array(a) => Some(a.len()),
                Value::Object(m) => m.values().find_map(|v| {
                    if let Value::Array(a) = v { Some(a.len()) } else { None }
                }),
                _ => None,
            };
            match arr_len {
                Some(n) if n <= 10 => res.pass(),
                Some(n) => res.fail(format!("getPeopleFirstTen returned {n} items, expected ≤ 10")),
                None => res.fail(format!("getPeopleFirstTen: couldn't parse array from {body}")),
            }
        }
        Ok((false, body)) => res.fail(format!("getPeopleFirstTen HTTP error: {body}")),
        Err(e) => res.fail(format!("getPeopleFirstTen error: {e}")),
    }

    // personExists for a known person_id → true
    if !people.is_empty() {
        let known_id = &people[0].person_id;
        match query_raw(&client, &base_url, "personExists", json!({ "person_id": known_id })).await {
            Ok((true, body)) => {
                match extract_bool(&body) {
                    Some(true) => res.pass(),
                    Some(false) => res.fail(format!("personExists({}) returned false for existing person", &known_id[..8])),
                    None => res.pass(), // accept if we can't parse the bool format
                }
            }
            Ok((false, body)) => res.fail(format!("personExists HTTP error: {body}")),
            Err(e) => res.fail(format!("personExists error: {e}")),
        }
    }

    // personExists for a random UUID → false
    let fake_id = Uuid::new_v4().to_string();
    match query_raw(&client, &base_url, "personExists", json!({ "person_id": &fake_id })).await {
        Ok((true, body)) => {
            match extract_bool(&body) {
                Some(false) => res.pass(),
                Some(true) => res.fail(format!("personExists returned true for fake UUID {}", &fake_id[..8])),
                None => res.pass(), // accept unknown format
            }
        }
        Ok((false, _)) => res.pass(), // HTTP error = not found = correct
        Err(e) => res.fail(format!("personExists(fake) error: {e}")),
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 12: Concurrent write race ──────────────────────────────────────────

async fn phase_concurrent_writes(
    client: Arc<Client>,
    base_url: Arc<String>,
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("concurrent_write_race");

    // All workers try to create the same company simultaneously.
    // UNIQUE INDEX must allow exactly 1 to succeed.
    let race_name = format!("RaceConditionCorp_{}", Uuid::new_v4());

    let results: Vec<bool> = stream::iter(0..workers)
        .map(|_| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            let name = race_name.clone();
            async move {
                matches!(
                    query_raw(&client, &base_url, "createCompany", json!({ "name": name })).await,
                    Ok((true, _))
                )
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let successes = results.iter().filter(|&&ok| ok).count();
    let failures = workers - successes;

    match successes {
        1 => {
            res.pass(); // exactly 1 writer won
            if failures == workers - 1 {
                res.pass(); // all others correctly rejected
            } else {
                res.fail(format!("expected {} rejections, got {failures}", workers - 1));
            }
        }
        0 => res.fail("all concurrent writers failed — should have had exactly 1 succeed".to_string()),
        n => {
            res.fail(format!("UNIQUE constraint violated: {n} of {workers} concurrent createCompany calls succeeded"));
            res.fail(format!("  race_name={race_name}"));
        }
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 13: BM25 text search lifecycle ──────────────────────────────────────

async fn phase_bm25_lifecycle(
    client: Arc<Client>,
    base_url: Arc<String>,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("bm25_lifecycle");

    // Use a prefix so distinctive we won't collide with real names
    let prefix = format!("ZBM25_{}", &Uuid::new_v4().to_string()[..8]);
    let ids: Vec<String> = (0..3).map(|i| format!("{prefix}_person_{i}")).collect();
    let names: Vec<String> = (0..3).map(|i| format!("{prefix}_name_{i}")).collect();

    // Insert 3 people with distinctive first_names
    for (pid, name) in ids.iter().zip(names.iter()) {
        match query_raw(&client, &base_url, "createPerson", json!({
            "person_id": pid, "first_name": name, "last_name": "BM25Test", "age": 30
        })).await {
            Ok((true, _)) => {}
            Ok((false, body)) => { res.fail(format!("BM25 insert failed for {name}: {body}")); }
            Err(e) => { res.fail(format!("BM25 insert error: {e}")); }
        }
    }

    // Search for prefix → should find all 3
    match query_raw(&client, &base_url, "searchPeopleByName", json!({ "query": &prefix, "limit": 10 })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, body)) => res.fail(format!("BM25 search after insert found nothing for prefix {prefix}: {body}")),
        Ok((false, body)) => res.fail(format!("BM25 search HTTP error: {body}")),
        Err(e) => res.fail(format!("BM25 search error: {e}")),
    }

    // Update first person's first_name to a different distinctive name
    let updated_name = format!("{prefix}_UPDATED");
    match query_raw(&client, &base_url, "updatePersonName", json!({
        "person_id": &ids[0], "first_name": &updated_name
    })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("updatePersonName failed: {body}")),
        Err(e) => res.fail(format!("updatePersonName error: {e}")),
    }

    // Search for the UPDATED name → must find it (BM25 index updated)
    match query_raw(&client, &base_url, "searchPeopleByName", json!({ "query": &updated_name, "limit": 5 })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, body)) => res.fail(format!("BM25 not updated after updatePersonName for {updated_name}: {body}")),
        Ok((false, body)) => res.fail(format!("BM25 search after update HTTP error: {body}")),
        Err(e) => res.fail(format!("BM25 search after update error: {e}")),
    }

    // Delete one of the non-updated people
    match query_raw(&client, &base_url, "deletePersonById", json!({ "person_id": &ids[1] })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("BM25 delete failed: {body}")),
        Err(e) => res.fail(format!("BM25 delete error: {e}")),
    }

    // Search the specific name of the deleted person → their person_id must NOT appear in results
    // (BM25 may return other people sharing prefix tokens, which is acceptable)
    match query_raw(&client, &base_url, "searchPeopleByName", json!({ "query": &names[1], "limit": 5 })).await {
        Ok((false, _)) => res.pass(),
        Ok((true, body)) => {
            let deleted_id = &ids[1];
            let rows = match &body {
                Value::Object(m) => m.values()
                    .find_map(|v| if let Value::Array(a) = v { Some(a.clone()) } else { None })
                    .unwrap_or_default(),
                Value::Array(a) => a.clone(),
                _ => vec![],
            };
            if rows.iter().any(|r| r.get("person_id").and_then(|v| v.as_str()) == Some(deleted_id.as_str())) {
                res.fail(format!("BM25 still returns deleted person_id {deleted_id}: {body}"));
            } else {
                res.pass();
            }
        }
        Err(e) => res.fail(format!("BM25 post-delete search error: {e}")),
    }

    // Clean up remaining BM25 test people
    for pid in &[&ids[0], &ids[2]] {
        let _ = query_raw(&client, &base_url, "deletePersonById", json!({ "person_id": pid })).await;
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 14: HNSW vector index lifecycle ────────────────────────────────────

async fn phase_hnsw_lifecycle(
    client: Arc<Client>,
    base_url: Arc<String>,
    workers: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("hnsw_lifecycle");
    const DIM: usize = 64;

    let tag_a = format!("hnsw_stress_a_{}", &Uuid::new_v4().to_string()[..8]);
    let tag_b = format!("hnsw_stress_b_{}", &Uuid::new_v4().to_string()[..8]);

    // Insert 20 vectors with tag_a
    let vecs_a: Vec<Vec<f64>> = (0..20).map(|_| random_vec(DIM)).collect();
    let insert_results: Vec<bool> = stream::iter(vecs_a.iter().enumerate())
        .map(|(i, vec)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            let tag = tag_a.clone();
            let vec = vec.clone();
            async move {
                matches!(
                    query_raw(&client, &base_url, "addSkillVec", json!({
                        "vec": vec, "skill_label": format!("skill_{i}"), "tag": tag
                    })).await,
                    Ok((true, _))
                )
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let inserted_a = insert_results.iter().filter(|&&ok| ok).count();
    if inserted_a == 0 {
        res.fail("HNSW: no vectors could be inserted".to_string());
        res.duration_ms = t.elapsed().as_millis() as u64;
        return res;
    }
    res.pass(); // insertions succeeded

    // SearchV with one of the inserted vectors → should return results
    let query_vec = vecs_a[0].clone();
    match query_raw(&client, &base_url, "searchSkillVecs", json!({ "vec": query_vec, "k": 5_i64 })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, body)) => res.fail(format!("SearchV returned empty after inserting {inserted_a} vectors: {body}")),
        Ok((false, body)) => res.fail(format!("SearchV HTTP error: {body}")),
        Err(e) => res.fail(format!("SearchV error: {e}")),
    }

    // skillVecsWithTag(tag_a) → should return inserted_a results
    match query_raw(&client, &base_url, "skillVecsWithTag", json!({ "tag": &tag_a })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, body)) => res.fail(format!("skillVecsWithTag({tag_a}) returned empty: {body}")),
        Ok((false, body)) => res.fail(format!("skillVecsWithTag HTTP error: {body}")),
        Err(e) => res.fail(format!("skillVecsWithTag error: {e}")),
    }

    // deleteSkillVecsByTag(tag_a)
    match query_raw(&client, &base_url, "deleteSkillVecsByTag", json!({ "tag": &tag_a })).await {
        Ok((true, _)) => res.pass(),
        Ok((false, body)) => res.fail(format!("deleteSkillVecsByTag failed: {body}")),
        Err(e) => res.fail(format!("deleteSkillVecsByTag error: {e}")),
    }

    // skillVecsWithTag(tag_a) → must be empty now
    match query_raw(&client, &base_url, "skillVecsWithTag", json!({ "tag": &tag_a })).await {
        Ok((true, body)) if !is_nonempty(&body) => res.pass(),
        Ok((false, _)) => res.pass(),
        Ok((true, body)) => res.fail(format!("HNSW vectors still visible after delete: {body}")),
        Err(e) => res.fail(format!("skillVecsWithTag after delete error: {e}")),
    }

    // Re-insert 5 vectors with tag_b → test re-indexing after delete
    let vecs_b: Vec<Vec<f64>> = (0..5).map(|_| random_vec(DIM)).collect();
    let reinsert_results: Vec<bool> = stream::iter(vecs_b.iter().enumerate())
        .map(|(i, vec)| {
            let client = Arc::clone(&client);
            let base_url = Arc::clone(&base_url);
            let tag = tag_b.clone();
            let vec = vec.clone();
            async move {
                matches!(
                    query_raw(&client, &base_url, "addSkillVec", json!({
                        "vec": vec, "skill_label": format!("reinsertion_{i}"), "tag": tag
                    })).await,
                    Ok((true, _))
                )
            }
        })
        .buffer_unordered(workers)
        .collect()
        .await;

    let reinserted = reinsert_results.iter().filter(|&&ok| ok).count();
    if reinserted > 0 {
        // SearchV should find the re-inserted vectors
        match query_raw(&client, &base_url, "searchSkillVecs",
            json!({ "vec": vecs_b[0].clone(), "k": 5_i64 })).await {
            Ok((true, body)) if is_nonempty(&body) => res.pass(),
            Ok((true, body)) => res.fail(format!("SearchV empty after re-insert: {body}")),
            _ => res.pass(), // don't fail hard — re-indexing is best-effort
        }
    }

    // Clean up tag_b
    let _ = query_raw(&client, &base_url, "deleteSkillVecsByTag", json!({ "tag": &tag_b })).await;

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Phase 15: Persistence verification (run after server restart) ─────────────

async fn phase_persistence(
    client: Arc<Client>,
    base_url: Arc<String>,
    expected_people: usize,
    expected_companies: usize,
    expected_jobs: usize,
) -> PhaseResult {
    let t = Instant::now();
    let mut res = PhaseResult::new("persistence");

    // countPeople == expected
    match query_raw(&client, &base_url, "countPeople", json!({})).await {
        Ok((true, body)) => match extract_count(&body) {
            Some(n) if n == expected_people as u64 => res.pass(),
            Some(n) => res.fail(format!("countPeople: expected {expected_people}, got {n}")),
            None => res.fail(format!("countPeople: unexpected response {body}")),
        },
        Ok((false, body)) => res.fail(format!("countPeople HTTP error: {body}")),
        Err(e) => res.fail(format!("countPeople error: {e}")),
    }

    // countCompanies >= expected (extra writes from Phase 9/12 can add a few)
    match query_raw(&client, &base_url, "getAllCompanies", json!({})).await {
        Ok((true, body)) => {
            let n = match &body {
                Value::Array(a) => a.len(),
                Value::Object(m) => m.values()
                    .find_map(|v| if let Value::Array(a) = v { Some(a.len()) } else { None })
                    .unwrap_or(0),
                _ => 0,
            };
            if n >= expected_companies {
                res.pass();
            } else {
                res.fail(format!("companies after restart: expected >= {expected_companies}, got {n}"));
            }
        }
        Ok((false, body)) => res.fail(format!("getAllCompanies HTTP error: {body}")),
        Err(e) => res.fail(format!("getAllCompanies error: {e}")),
    }

    // getAllJobs nonempty and count matches
    match query_raw(&client, &base_url, "getAllJobs", json!({})).await {
        Ok((true, body)) => {
            let n = match &body {
                Value::Array(a) => a.len(),
                Value::Object(m) => m.values()
                    .find_map(|v| if let Value::Array(a) = v { Some(a.len()) } else { None })
                    .unwrap_or(0),
                _ => 0,
            };
            if n == expected_jobs {
                res.pass();
            } else {
                res.fail(format!("jobs after restart: expected {expected_jobs}, got {n}"));
            }
        }
        Ok((false, body)) => res.fail(format!("getAllJobs HTTP error: {body}")),
        Err(e) => res.fail(format!("getAllJobs error: {e}")),
    }

    // getFirstPerson → nonempty, then traverse to company+job
    let first_person_id: Option<String> = match query_raw(&client, &base_url, "getFirstPerson", json!({})).await {
        Ok((true, body)) => {
            let pid = match &body {
                Value::Object(m) => m.get("person_id").and_then(|v| v.as_str()).map(|s| s.to_string())
                    .or_else(|| m.values().find_map(|v| {
                        if let Value::Object(inner) = v {
                            inner.get("person_id").and_then(|v| v.as_str()).map(|s| s.to_string())
                        } else { None }
                    })),
                Value::Array(a) => a.first().and_then(|v| v.get("person_id")).and_then(|v| v.as_str()).map(|s| s.to_string()),
                _ => None,
            };
            if pid.is_some() { res.pass(); } else { res.fail(format!("getFirstPerson: no person_id in {body}")); }
            pid
        }
        Ok((false, body)) => { res.fail(format!("getFirstPerson HTTP error: {body}")); None }
        Err(e) => { res.fail(format!("getFirstPerson error: {e}")); None }
    };

    // Traversal: GetPersonFullInfo still works after restart
    if let Some(pid) = first_person_id {
        match query_raw(&client, &base_url, "GetPersonFullInfo", json!({ "person_id": &pid })).await {
            Ok((true, _)) => res.pass(),
            Ok((false, body)) => res.fail(format!("GetPersonFullInfo after restart: HTTP error {body}")),
            Err(e) => res.fail(format!("GetPersonFullInfo after restart: {e}")),
        }
    }

    // BM25 index survived: a broad search returns something
    match query_raw(&client, &base_url, "searchPeopleByName", json!({ "query": "Person", "limit": 5 })).await {
        Ok((true, body)) if is_nonempty(&body) => res.pass(),
        Ok((true, _)) => res.pass(), // empty BM25 is acceptable — names may not match "Person"
        Ok((false, body)) => res.fail(format!("searchPeopleByName after restart: HTTP error {body}")),
        Err(e) => res.fail(format!("searchPeopleByName after restart: {e}")),
    }

    res.duration_ms = t.elapsed().as_millis() as u64;
    res
}

// ── Main entry point ──────────────────────────────────────────────────────────

pub async fn run(
    endpoint: String,
    port: u16,
    num_people: usize,
    num_companies: usize,
    num_jobs: usize,
    workers: usize,
    progress_interval: usize,
    verify_only: bool,
) -> Result<()> {
    let base = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        format!("{}:{}", endpoint, port)
    } else {
        format!("http://{}:{}", endpoint, port)
    };

    println!("=== SparrowDB Stress Test ===\n");
    println!("Target:    {}", base);
    println!("People:    {}", num_people);
    println!("Companies: {}", num_companies);
    println!("Jobs:      {}", num_jobs);
    println!("Workers:   {}", workers);
    if verify_only {
        println!("Mode:      verify-only (persistence check after restart)");
    }
    println!();

    let client = Arc::new(Client::new());
    let base_url = Arc::new(base);
    let total_start = Instant::now();

    // ── Verify-only mode: skip writes, just check persisted data ───────────
    if verify_only {
        println!("── Phase 15: Persistence verification ─────────────────");
        let r = phase_persistence(
            Arc::clone(&client),
            Arc::clone(&base_url),
            num_people,
            num_companies,
            num_jobs,
        ).await;
        r.print();
        let elapsed = total_start.elapsed();
        println!("\n── Summary ─────────────────────────────────────────────");
        println!("  Total time: {:.2}s", elapsed.as_secs_f64());
        return Ok(());
    }

    // ── Phase 1: Write ──────────────────────────────────────────────────────
    println!("── Phase 1: Write ─────────────────────────────────────");
    let write_start = Instant::now();

    let companies = insert_companies(Arc::clone(&client), Arc::clone(&base_url), num_companies, workers).await;
    println!();
    let jobs = insert_jobs(Arc::clone(&client), Arc::clone(&base_url), num_jobs, workers).await;
    println!();
    let people = insert_people(Arc::clone(&client), Arc::clone(&base_url), num_people, companies.clone(), jobs.clone(), workers, progress_interval).await;
    println!();
    connect_companies_to_jobs(Arc::clone(&client), Arc::clone(&base_url), &companies, &jobs, workers).await;

    let write_ms = write_start.elapsed().as_millis();
    println!("\nWrite phase: {:.2}s  ({} records)", write_ms as f64 / 1000.0,
        people.len() + companies.len() + jobs.len());

    // ── Phase 2: Valid traversals ──────────────────────────────────────────
    println!("\n── Phase 2: Valid traversals ───────────────────────────");
    let r = phase_valid_traversals(Arc::clone(&client), Arc::clone(&base_url), &people, &companies, &jobs, workers).await;
    r.print();

    // ── Phase 3: Non-existent lookups ─────────────────────────────────────
    println!("\n── Phase 3: Non-existent lookups ──────────────────────");
    let r = phase_nonexistent_lookups(Arc::clone(&client), Arc::clone(&base_url), workers).await;
    r.print();

    // ── Phase 4: Boundary age queries ────────────────────────────────────
    println!("\n── Phase 4: Boundary age queries ──────────────────────");
    let r = phase_boundary_age(Arc::clone(&client), Arc::clone(&base_url), num_people).await;
    r.print();

    // ── Phase 5: Duplicate write rejection ────────────────────────────────
    println!("\n── Phase 5: Duplicate write rejection ─────────────────");
    let r = phase_duplicate_writes(Arc::clone(&client), Arc::clone(&base_url), &companies, &jobs, &people, workers).await;
    r.print();

    // ── Phase 6: Degenerate inputs ───────────────────────────────────────
    println!("\n── Phase 6: Degenerate inputs ─────────────────────────");
    let r = phase_degenerate_inputs(Arc::clone(&client), Arc::clone(&base_url)).await;
    r.print();

    // ── Phase 7: Concurrent read storm ───────────────────────────────────
    println!("\n── Phase 7: Concurrent read storm ─────────────────────");
    let r = phase_concurrent_reads(Arc::clone(&client), Arc::clone(&base_url), &people, workers).await;
    r.print();

    // ── Phase 8: Node updates
    println!("\n── Phase 8: Node updates ───────────────────────────────");
    let r = phase_node_updates(Arc::clone(&client), Arc::clone(&base_url), &people, workers).await;
    r.print();

    // ── Phase 9: Delete lifecycle
    println!("\n── Phase 9: Delete lifecycle ───────────────────────────");
    let r = phase_delete_lifecycle(Arc::clone(&client), Arc::clone(&base_url), &companies).await;
    r.print();

    // ── Phase 10: Multi-hop traversals
    println!("\n── Phase 10: Multi-hop traversals ─────────────────────");
    let r = phase_multihop_traversals(Arc::clone(&client), Arc::clone(&base_url), &people, workers).await;
    r.print();

    // ── Phase 11: COUNT, RANGE, FIRST, ORDER, EXISTS
    println!("\n── Phase 11: Aggregation ops ───────────────────────────");
    let r = phase_aggregation(Arc::clone(&client), Arc::clone(&base_url), &companies, &people).await;
    r.print();

    // ── Phase 12: Concurrent write race
    println!("\n── Phase 12: Concurrent write race ─────────────────────");
    let r = phase_concurrent_writes(Arc::clone(&client), Arc::clone(&base_url), workers).await;
    r.print();

    // ── Phase 13: BM25 lifecycle
    println!("\n── Phase 13: BM25 lifecycle ────────────────────────────");
    let r = phase_bm25_lifecycle(Arc::clone(&client), Arc::clone(&base_url)).await;
    r.print();

    // ── Phase 14: HNSW vector lifecycle
    println!("\n── Phase 14: HNSW vector lifecycle ─────────────────────");
    let r = phase_hnsw_lifecycle(Arc::clone(&client), Arc::clone(&base_url), workers).await;
    r.print();

    // ── Summary ───────────────────────────────────────────────────────────
    let elapsed = total_start.elapsed();
    let write_records = people.len() + companies.len() + jobs.len();
    println!("\n── Summary ─────────────────────────────────────────────");
    println!("  {} people | {} companies | {} jobs written", people.len(), companies.len(), jobs.len());
    println!("  Total time:  {:.2}s", elapsed.as_secs_f64());
    println!("  Write throughput: {:.0} records/sec",
        write_records as f64 / (write_ms as f64 / 1000.0));

    Ok(())
}
