//! URL and URLSearchParams backed by the pinned WHATWG URL parser.

use std::{cell::RefCell, mem::size_of, rc::Rc};

use rquickjs::atom::PredefinedAtom;
use rquickjs::class::Trace;
use rquickjs::function::{Opt, This};
use rquickjs::object::Property;
use rquickjs::{Array, Class, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Result, Value};

use url::{Url as ParsedUrl, form_urlencoded, quirks};

use super::encoding::usv_string;
use super::web::{Charge, constructor, is_nullish};

type Pair = (String, String);

pub(super) fn install<'js>(ctx: &Ctx<'js>, globals: &Object<'js>) -> Result<()> {
    let parameters = constructor::<URLSearchParams, _, _>(ctx, move |ctx, input| {
        URLSearchParams::standalone(&ctx, input)
    })?;
    let prototype: Object = parameters.get("prototype")?;
    let entries: Function = prototype.get("entries")?;
    prototype.prop(
        PredefinedAtom::SymbolIterator,
        Property::from(entries).configurable().writable(),
    )?;

    let url = constructor::<Url<'js>, _, _>(ctx, Url::new)?;
    url.set_length(2)?;
    let can_parse = Function::new(ctx.clone(), can_parse)?
        .with_name("canParse")?
        .with_length(2)?;
    url.prop(
        "canParse",
        Property::from(can_parse).configurable().writable(),
    )?;
    globals.set("URLSearchParams", parameters)?;
    globals.set("URL", url)
}

pub(super) fn parse<'js>(
    ctx: &Ctx<'js>,
    input: Value<'js>,
    base: Option<Value<'js>>,
) -> Result<ParsedUrl> {
    let (input, base) = url_inputs(ctx, input, base)?;
    let base = base
        .as_deref()
        .map(ParsedUrl::parse)
        .transpose()
        .map_err(|_| Exception::throw_type(ctx, "Invalid base URL"))?;
    ParsedUrl::options()
        .base_url(base.as_ref())
        .parse(&input)
        .map_err(|_| Exception::throw_type(ctx, "Invalid URL"))
}

fn url_inputs<'js>(
    ctx: &Ctx<'js>,
    input: Value<'js>,
    base: Option<Value<'js>>,
) -> Result<(String, Option<String>)> {
    let input = usv_string(ctx, input)?;
    let base = base
        .filter(|value| !value.is_undefined())
        .map(|value| usv_string(ctx, value))
        .transpose()?;
    Ok((input, base))
}

fn can_parse<'js>(
    ctx: Ctx<'js>,
    Opt(input): Opt<Value<'js>>,
    Opt(base): Opt<Value<'js>>,
) -> Result<bool> {
    let input = input.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
    let (input, base) = url_inputs(&ctx, input, base)?;
    let Ok(base) = base.as_deref().map(ParsedUrl::parse).transpose() else {
        return Ok(false);
    };
    Ok(ParsedUrl::options()
        .base_url(base.as_ref())
        .parse(&input)
        .is_ok())
}

/// Serialize a URLSearchParams body without consulting user-overridden methods.
pub(super) fn form_query<'js>(value: &Value<'js>) -> Result<Option<String>> {
    value
        .as_object()
        .and_then(|object| object.as_class::<URLSearchParams>())
        .map(|params| {
            let params = params.try_borrow()?;
            Ok(params.serialize())
        })
        .transpose()
}

fn parse_pairs(ctx: &Ctx<'_>, query: &str) -> Result<(Vec<Pair>, Charge)> {
    collect_pairs(
        ctx,
        form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .map(Ok),
    )
}

fn collect_pairs(
    ctx: &Ctx<'_>,
    entries: impl IntoIterator<Item = Result<Pair>>,
) -> Result<(Vec<Pair>, Charge)> {
    let mut collector = PairCollector::new(ctx)?;
    for pair in entries {
        collector.push(ctx, pair?)?;
    }
    Ok(collector.finish())
}

struct PairCollector {
    pairs: Vec<Pair>,
    contents: usize,
    charge: Charge,
}

impl PairCollector {
    fn new(ctx: &Ctx<'_>) -> Result<Self> {
        Ok(Self {
            pairs: Vec::new(),
            contents: 0,
            charge: Charge::new(ctx)?,
        })
    }

    fn push(&mut self, ctx: &Ctx<'_>, pair: Pair) -> Result<()> {
        let contents = self
            .contents
            .saturating_add(pair.0.capacity())
            .saturating_add(pair.1.capacity());
        let capacity = self.pairs.capacity();
        let next = if self.pairs.len() == capacity {
            capacity.saturating_mul(2).max(4)
        } else {
            capacity
        };
        self.charge.resize(
            ctx,
            contents.saturating_add(next.saturating_mul(size_of::<Pair>())),
        )?;
        if next != capacity {
            self.pairs.reserve_exact(next - self.pairs.len());
        }
        self.pairs.push(pair);
        self.contents = contents;
        Ok(())
    }

    fn finish(self) -> (Vec<Pair>, Charge) {
        (self.pairs, self.charge)
    }
}

fn serialize_pairs(pairs: &[Pair]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(pairs);
    serializer.finish()
}

fn pair_bytes(pairs: &Vec<Pair>) -> usize {
    pairs.iter().fold(
        pairs.capacity().saturating_mul(size_of::<Pair>()),
        |bytes, (key, value)| {
            bytes
                .saturating_add(key.capacity())
                .saturating_add(value.capacity())
        },
    )
}

fn has_iterator(value: &Object<'_>) -> Result<bool> {
    let method: Value = value.get(PredefinedAtom::SymbolIterator)?;
    Ok(!is_nullish(&method))
}

fn map_iterable<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
    mapper: Function<'js>,
) -> Result<Array<'js>> {
    let array: Function = ctx.globals().get("Array")?;
    let from: Function = array.get("from")?;
    from.call((This(array), value, mapper))
}

fn read_pair<'js>(ctx: &Ctx<'js>, entry: Value<'js>) -> Result<Pair> {
    if !entry.as_object().map_or(Ok(false), has_iterator)? {
        return Err(Exception::throw_type(ctx, "Expected an iterable pair"));
    }
    let convert = Function::new(ctx.clone(), |ctx: Ctx<'js>, value| usv_string(&ctx, value))?;
    let values = map_iterable(ctx, entry, convert)?;
    if values.len() != 2 {
        return Err(Exception::throw_type(ctx, "Expected a name and value"));
    }
    Ok((values.get(0)?, values.get(1)?))
}

fn iterable_pairs<'js>(ctx: &Ctx<'js>, init: Value<'js>) -> Result<(Vec<Pair>, Charge)> {
    let collector = Rc::new(RefCell::new(Some(PairCollector::new(ctx)?)));
    let mapped = Rc::clone(&collector);
    let mapper = Function::new(ctx.clone(), move |ctx: Ctx<'js>, entry: Value<'js>| {
        let pair = read_pair(&ctx, entry)?;
        let mut state = mapped.borrow_mut();
        let state = state
            .as_mut()
            .ok_or_else(|| Exception::throw_type(&ctx, "Parameter iteration has ended"))?;
        state.push(&ctx, pair)
    })?;
    // QuickJS calls the native mapper before advancing the iterator and closes
    // the iterator if coercion or the allocation limit interrupts mapping.
    let result = map_iterable(ctx, init, mapper);
    let collected = collector.borrow_mut().take();
    result?;
    collected
        .map(PairCollector::finish)
        .ok_or_else(|| Exception::throw_internal(ctx, "Parameter collector is unavailable"))
}

fn initial_pairs<'js>(ctx: &Ctx<'js>, init: Option<Value<'js>>) -> Result<(Vec<Pair>, Charge)> {
    let Some(init) = init.filter(|value| !is_nullish(value)) else {
        return Ok((Vec::new(), Charge::new(ctx)?));
    };
    if let Some(object) = init.as_object().filter(|_| !init.is_function()) {
        if !has_iterator(object)? {
            let entries = object.keys::<rquickjs::String>().map(|key| {
                let key = key?;
                let value = object.get::<_, Value>(key.clone())?;
                Ok((usv_string(ctx, key.into_value())?, usv_string(ctx, value)?))
            });
            return collect_pairs(ctx, entries);
        }
        return iterable_pairs(ctx, init);
    }
    let query = usv_string(ctx, init)?;
    parse_pairs(ctx, query.strip_prefix('?').unwrap_or(&query))
}

struct UrlState {
    url: ParsedUrl,
    pairs: Vec<Pair>,
    charge: Charge,
}

impl UrlState {
    fn replace(&mut self, ctx: &Ctx<'_>, url: ParsedUrl, pairs: Vec<Pair>) -> Result<()> {
        self.charge
            .resize(ctx, url.as_str().len().saturating_add(pair_bytes(&pairs)))?;
        self.url = url;
        self.pairs = pairs;
        Ok(())
    }
}

enum ParamSource {
    Standalone { pairs: Vec<Pair>, charge: Charge },
    Url(Rc<RefCell<UrlState>>),
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct URLSearchParams {
    #[qjs(skip_trace)]
    source: ParamSource,
}

impl URLSearchParams {
    fn standalone<'js>(ctx: &Ctx<'js>, Opt(input): Opt<Value<'js>>) -> Result<Self> {
        let (pairs, charge) = initial_pairs(ctx, input)?;
        Ok(Self {
            source: ParamSource::Standalone { pairs, charge },
        })
    }

    fn read<R>(&self, read: impl FnOnce(&[Pair]) -> R) -> R {
        match &self.source {
            ParamSource::Standalone { pairs, .. } => read(pairs),
            ParamSource::Url(owner) => read(&owner.borrow().pairs),
        }
    }

    fn edit(&mut self, ctx: &Ctx<'_>, edit: impl FnOnce(&mut Vec<Pair>)) -> Result<()> {
        match &mut self.source {
            ParamSource::Standalone { pairs, charge } => {
                let mut next = pairs.clone();
                edit(&mut next);
                charge.resize(ctx, pair_bytes(&next))?;
                *pairs = next;
            }
            ParamSource::Url(owner) => {
                let mut owner = owner.borrow_mut();
                let mut pairs = owner.pairs.clone();
                edit(&mut pairs);
                let mut url = owner.url.clone();
                let query = serialize_pairs(&pairs);
                // Empty lists remove the query; nonempty form data is already percent encoded.
                url.set_query((!query.is_empty()).then_some(query.as_str()));
                owner.replace(ctx, url, pairs)?;
            }
        }
        Ok(())
    }
}

fn key_and_value<'js>(
    ctx: &Ctx<'js>,
    Opt(key): Opt<Value<'js>>,
    Opt(value): Opt<Value<'js>>,
) -> Result<Pair> {
    Ok((
        usv_string(
            ctx,
            key.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
        )?,
        usv_string(
            ctx,
            value.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
        )?,
    ))
}

fn optional_filter<'js>(
    ctx: &Ctx<'js>,
    Opt(key): Opt<Value<'js>>,
    Opt(value): Opt<Value<'js>>,
) -> Result<(String, Option<String>)> {
    let key = usv_string(
        ctx,
        key.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
    )?;
    let filter = value
        .filter(|value| !value.is_undefined())
        .map(|value| usv_string(ctx, value))
        .transpose()?;
    Ok((key, filter))
}

#[rquickjs::methods]
impl URLSearchParams {
    #[qjs(get)]
    fn size(&self) -> usize {
        self.read(<[Pair]>::len)
    }

    fn append<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Opt<Value<'js>>,
        value: Opt<Value<'js>>,
    ) -> Result<()> {
        let entry = key_and_value(&ctx, key, value)?;
        this.try_borrow_mut()?
            .edit(&ctx, move |pairs| pairs.push(entry))
    }

    fn delete<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Opt<Value<'js>>,
        value: Opt<Value<'js>>,
    ) -> Result<()> {
        let (key, filter) = optional_filter(&ctx, key, value)?;
        this.try_borrow_mut()?.edit(&ctx, |pairs| {
            pairs.retain(|(name, value)| {
                *name != key || filter.as_ref().is_some_and(|filter| filter != value)
            })
        })
    }

    fn get<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        Opt(key): Opt<Value<'js>>,
    ) -> Result<Value<'js>> {
        let key = usv_string(
            &ctx,
            key.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
        )?;
        let value = this.try_borrow()?.read(|pairs| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        });
        match value {
            Some(value) => value.into_js(&ctx),
            None => Ok(Value::new_null(ctx)),
        }
    }

    #[qjs(rename = "getAll")]
    fn get_all<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        Opt(key): Opt<Value<'js>>,
    ) -> Result<Vec<String>> {
        let key = usv_string(
            &ctx,
            key.unwrap_or_else(|| Value::new_undefined(ctx.clone())),
        )?;
        Ok(this.try_borrow()?.read(|pairs| {
            pairs
                .iter()
                .filter(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
                .collect()
        }))
    }

    fn has<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Opt<Value<'js>>,
        value: Opt<Value<'js>>,
    ) -> Result<bool> {
        let (key, filter) = optional_filter(&ctx, key, value)?;
        Ok(this.try_borrow()?.read(|pairs| {
            pairs.iter().any(|(name, value)| {
                *name == key && filter.as_ref().is_none_or(|filter| filter == value)
            })
        }))
    }

    fn set<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        key: Opt<Value<'js>>,
        value: Opt<Value<'js>>,
    ) -> Result<()> {
        let (key, value) = key_and_value(&ctx, key, value)?;
        this.try_borrow_mut()?.edit(&ctx, |pairs| {
            if let Some(index) = pairs.iter().position(|(name, _)| *name == key) {
                pairs[index].1 = value;
                let mut source = 0;
                pairs.retain(|(name, _)| {
                    let retain = source <= index || *name != key;
                    source += 1;
                    retain
                });
            } else {
                pairs.push((key, value));
            }
        })
    }

    fn sort<'js>(This(this): This<Class<'js, Self>>, ctx: Ctx<'js>) -> Result<()> {
        this.try_borrow_mut()?.edit(&ctx, |pairs| {
            pairs.sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()))
        })
    }

    fn entries<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
    ) -> Result<Class<'js, SearchIterator<'js>>> {
        SearchIterator::new(ctx, this, Iteration::Entries)
    }

    fn keys<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
    ) -> Result<Class<'js, SearchIterator<'js>>> {
        SearchIterator::new(ctx, this, Iteration::Keys)
    }

    fn values<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
    ) -> Result<Class<'js, SearchIterator<'js>>> {
        SearchIterator::new(ctx, this, Iteration::Values)
    }

    #[qjs(rename = "forEach")]
    fn for_each<'js>(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        callback: Function<'js>,
        Opt(this_arg): Opt<Value<'js>>,
    ) -> Result<()> {
        let this_arg = this_arg.unwrap_or_else(|| Value::new_undefined(ctx));
        let mut position = 0;
        loop {
            let entry = this
                .try_borrow()?
                .read(|pairs| pairs.get(position).cloned());
            let Some((key, value)) = entry else {
                return Ok(());
            };
            // Callbacks may mutate this same URLSearchParams instance.
            callback.call::<_, ()>((This(this_arg.clone()), value, key, this.clone()))?;
            position += 1;
        }
    }

    #[qjs(rename = "toString")]
    fn serialize(&self) -> String {
        self.read(serialize_pairs)
    }
}

#[derive(Clone, Copy)]
enum Iteration {
    Entries,
    Keys,
    Values,
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
struct SearchIterator<'js> {
    source: Option<Class<'js, URLSearchParams>>,
    position: usize,
    #[qjs(skip_trace)]
    kind: Iteration,
}

#[rquickjs::methods]
impl<'js> SearchIterator<'js> {
    #[qjs(skip)]
    fn new(
        ctx: Ctx<'js>,
        source: Class<'js, URLSearchParams>,
        kind: Iteration,
    ) -> Result<Class<'js, Self>> {
        Class::instance(
            ctx,
            Self {
                source: Some(source),
                position: 0,
                kind,
            },
        )
    }

    #[qjs(rename = PredefinedAtom::SymbolIterator)]
    fn identity(This(this): This<Class<'js, Self>>) -> Class<'js, Self> {
        this
    }

    fn next(&mut self, ctx: Ctx<'js>) -> Result<Object<'js>> {
        let entry = self
            .source
            .as_ref()
            .map(|source| {
                source
                    .try_borrow()
                    .map(|source| source.read(|pairs| pairs.get(self.position).cloned()))
            })
            .transpose()?
            .flatten();
        let result = Object::new(ctx.clone())?;
        result.prop(
            "done",
            Property::from(entry.is_none())
                .enumerable()
                .writable()
                .configurable(),
        )?;
        if let Some((key, value)) = entry {
            self.position += 1;
            match self.kind {
                Iteration::Entries => {
                    let pair = Array::new(ctx)?;
                    pair.as_object().prop(
                        0,
                        Property::from(key).enumerable().configurable().writable(),
                    )?;
                    pair.as_object().prop(
                        1,
                        Property::from(value).enumerable().configurable().writable(),
                    )?;
                    result.prop(
                        "value",
                        Property::from(pair).enumerable().writable().configurable(),
                    )?;
                }
                Iteration::Keys | Iteration::Values => {
                    let item = if matches!(self.kind, Iteration::Keys) {
                        key
                    } else {
                        value
                    };
                    result.prop(
                        "value",
                        Property::from(item).enumerable().writable().configurable(),
                    )?;
                }
            }
        } else {
            self.source = None;
            result.prop(
                "value",
                Property::from(Value::new_undefined(ctx))
                    .enumerable()
                    .writable()
                    .configurable(),
            )?;
        }
        Ok(result)
    }
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "URL")]
struct Url<'js> {
    #[qjs(skip_trace)]
    state: Rc<RefCell<UrlState>>,
    parameters: Class<'js, URLSearchParams>,
}

enum Part {
    Href,
    Protocol,
    Username,
    Password,
    Host,
    Hostname,
    Port,
    Pathname,
    Search,
    Hash,
}

impl<'js> Url<'js> {
    fn new(ctx: Ctx<'js>, Opt(input): Opt<Value<'js>>, Opt(base): Opt<Value<'js>>) -> Result<Self> {
        let input = input.unwrap_or_else(|| Value::new_undefined(ctx.clone()));
        let url = parse(&ctx, input, base)?;
        let (pairs, mut charge) = parse_pairs(&ctx, url.query().unwrap_or_default())?;
        charge.resize(&ctx, url.as_str().len().saturating_add(pair_bytes(&pairs)))?;
        let state = Rc::new(RefCell::new(UrlState { url, pairs, charge }));
        let parameters = Class::instance(
            ctx,
            URLSearchParams {
                source: ParamSource::Url(Rc::clone(&state)),
            },
        )?;
        Ok(Self { state, parameters })
    }

    fn set_part(
        this: Class<'js, Self>,
        ctx: Ctx<'js>,
        part: Part,
        input: Value<'js>,
    ) -> Result<()> {
        // Converting to a USVString may invoke user code. Borrow the URL afterward.
        let value = usv_string(&ctx, input)?;
        let object = this.try_borrow()?;
        let mut state = object.state.borrow_mut();
        let mut url = state.url.clone();
        match part {
            Part::Href => quirks::set_href(&mut url, &value)
                .map_err(|_| Exception::throw_type(&ctx, "Invalid URL"))?,
            Part::Protocol => {
                let _ = quirks::set_protocol(&mut url, &value);
            }
            Part::Username => {
                let _ = quirks::set_username(&mut url, &value);
            }
            Part::Password => {
                let _ = quirks::set_password(&mut url, &value);
            }
            Part::Host => {
                let _ = quirks::set_host(&mut url, &value);
            }
            Part::Hostname => {
                let _ = quirks::set_hostname(&mut url, &value);
            }
            Part::Port => {
                let _ = quirks::set_port(&mut url, &value);
            }
            Part::Pathname => quirks::set_pathname(&mut url, &value),
            Part::Search => quirks::set_search(&mut url, &value),
            Part::Hash => quirks::set_hash(&mut url, &value),
        }
        if matches!(part, Part::Href | Part::Search) {
            let (pairs, charge) = parse_pairs(&ctx, url.query().unwrap_or_default())?;
            // Transfer the new allocation before the combined URL charge changes.
            drop(charge);
            state.replace(&ctx, url, pairs)
        } else {
            let bytes = url.as_str().len().saturating_add(pair_bytes(&state.pairs));
            state.charge.resize(&ctx, bytes)?;
            state.url = url;
            Ok(())
        }
    }
}

#[rquickjs::methods]
impl<'js> Url<'js> {
    #[qjs(get)]
    fn origin(&self) -> String {
        quirks::origin(&self.state.borrow().url)
    }

    #[qjs(get, rename = "searchParams")]
    fn search_params(&self) -> Class<'js, URLSearchParams> {
        self.parameters.clone()
    }

    #[qjs(rename = "toString")]
    fn serialize(&self) -> String {
        self.href()
    }

    #[qjs(rename = "toJSON")]
    fn to_json(&self) -> String {
        self.href()
    }

    #[qjs(get, rename = "href")]
    fn href(&self) -> String {
        quirks::href(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "href")]
    fn set_href(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Href, value)
    }

    #[qjs(get, rename = "protocol")]
    fn protocol(&self) -> String {
        quirks::protocol(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "protocol")]
    fn set_protocol(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Protocol, value)
    }

    #[qjs(get, rename = "username")]
    fn username(&self) -> String {
        quirks::username(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "username")]
    fn set_username(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Username, value)
    }

    #[qjs(get, rename = "password")]
    fn password(&self) -> String {
        quirks::password(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "password")]
    fn set_password(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Password, value)
    }

    #[qjs(get, rename = "host")]
    fn host(&self) -> String {
        quirks::host(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "host")]
    fn set_host(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Host, value)
    }

    #[qjs(get, rename = "hostname")]
    fn hostname(&self) -> String {
        quirks::hostname(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "hostname")]
    fn set_hostname(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Hostname, value)
    }

    #[qjs(get, rename = "port")]
    fn port(&self) -> String {
        quirks::port(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "port")]
    fn set_port(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Port, value)
    }

    #[qjs(get, rename = "pathname")]
    fn pathname(&self) -> String {
        quirks::pathname(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "pathname")]
    fn set_pathname(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Pathname, value)
    }

    #[qjs(get, rename = "search")]
    fn search(&self) -> String {
        quirks::search(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "search")]
    fn set_search(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Search, value)
    }

    #[qjs(get, rename = "hash")]
    fn hash(&self) -> String {
        quirks::hash(&self.state.borrow().url).to_owned()
    }

    #[qjs(set, rename = "hash")]
    fn set_hash(
        This(this): This<Class<'js, Self>>,
        ctx: Ctx<'js>,
        value: Value<'js>,
    ) -> Result<()> {
        Self::set_part(this, ctx, Part::Hash, value)
    }
}

pub(super) const DECLARATIONS: &str = r#"
type URLSearchParam = readonly [string, string];
type URLSearchParamsInit = string | readonly URLSearchParam[] | Iterable<URLSearchParam> | Record<string, string>;

declare class URLSearchParams implements Iterable<[string, string]> {
  constructor(init?: URLSearchParamsInit);
  readonly size: number;
  append(name: string, value: string): void;
  delete(name: string, value?: string): void;
  get(name: string): string | null;
  getAll(name: string): string[];
  has(name: string, value?: string): boolean;
  set(name: string, value: string): void;
  sort(): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  forEach(callback: (value: string, name: string, params: URLSearchParams) => void, thisArg?: unknown): void;
  toString(): string;
  [Symbol.iterator](): IterableIterator<[string, string]>;
}

declare class URL {
  constructor(input: string | URL, base?: string | URL);
  static canParse(input: string | URL, base?: string | URL): boolean;
  href: string;
  readonly origin: string;
  protocol: string;
  username: string;
  password: string;
  host: string;
  hostname: string;
  port: string;
  pathname: string;
  search: string;
  readonly searchParams: URLSearchParams;
  hash: string;
  toJSON(): string;
  toString(): string;
}
"#;
