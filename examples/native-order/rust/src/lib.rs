use std::sync::Arc;

use impasto::{
    CancelToken, ImpastoType, PluginContext, PluginInitError, PluginRegistrar, TypedSendError,
    TypedSink,
};
use impasto_http::{HttpClient, HttpError, HttpRequest};

#[derive(Clone, ImpastoType)]
pub struct OrderSummary {
    pub id: String,
    pub total_cents: u32,
}

#[impasto::object]
pub struct OrderStore {
    orders: Vec<OrderSummary>,
}

#[impasto::export]
impl OrderStore {
    pub async fn query(&self) -> Vec<OrderSummary> {
        self.orders.clone()
    }

    pub async fn watch(
        &self,
        mut sink: TypedSink<OrderSummary>,
        cancel: CancelToken,
    ) -> Result<(), TypedSendError> {
        for order in &self.orders {
            if cancel.is_cancelled() {
                break;
            }
            sink.send(order.clone()).await?;
        }
        Ok(())
    }
}

#[impasto::module(specifier = "@app/order", init = from_context)]
pub struct OrderModule {
    http: Arc<HttpClient>,
}

impl OrderModule {
    fn from_context(ctx: &PluginContext) -> Result<Self, PluginInitError> {
        Ok(Self {
            http: impasto_http::client(ctx).map_err(PluginInitError::new)?,
        })
    }
}

#[impasto::export]
impl OrderModule {
    #[impasto::method(resource)]
    pub async fn open_store() -> OrderStore {
        OrderStore {
            orders: vec![OrderSummary {
                id: "order-1".into(),
                total_cents: 2599,
            }],
        }
    }

    pub async fn fetch_status(&self, url: String) -> Result<u16, HttpError> {
        Ok(self.http.send(HttpRequest::get(url)).await?.status)
    }
}

pub fn register(registrar: &mut PluginRegistrar) {
    registrar.add(OrderModule::plugin());
}
