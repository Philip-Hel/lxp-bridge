use crate::prelude::*;

use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, Publish, QoS};

// Message {{{
#[derive(Debug, Clone)]
pub struct Message {
    pub topic: String,
    pub payload: String,
}

impl Message {
    pub fn payload_int(&self) -> Result<u16> {
        match self.payload.parse() {
            Ok(i) => Ok(i),
            Err(err) => Err(anyhow!("payload_int: {}", err)),
        }
    }

    pub fn payload_bool(&self) -> bool {
        matches!(
            self.payload.to_ascii_lowercase().as_str(),
            "1" | "t" | "true" | "on" | "y" | "yes"
        )
    }
} // }}}

pub type MessageSender = broadcast::Sender<Message>;

pub struct Mqtt {
    config: Rc<Config>,
    from_coordinator: MessageSender,
    to_coordinator: MessageSender,
}

impl Mqtt {
    pub fn new(
        config: Rc<Config>,
        from_coordinator: MessageSender,
        to_coordinator: MessageSender,
    ) -> Self {
        Self {
            config,
            from_coordinator,
            to_coordinator,
        }
    }

    pub async fn start(&self) -> Result<()> {
        let m = &self.config.mqtt;

        let mut options = MqttOptions::new("lxp-bridge", &m.host, m.port);

        options.set_keep_alive(60);
        if let (Some(u), Some(p)) = (&m.username, &m.password) {
            options.set_credentials(u, p);
        }

        info!("connecting to mqtt at {}:{}", &m.host, m.port);

        let (client, eventloop) = AsyncClient::new(options, 10);

        info!("mqtt connected!");

        client
            .subscribe(
                format!(
                    "{}/cmd/{}/#",
                    self.config.mqtt.namespace, self.config.inverter.datalog
                ),
                QoS::AtMostOnce,
            )
            .await?;

        futures::try_join!(self.receiver(eventloop), self.sender(client))?;

        Ok(())
    }

    // mqtt -> coordinator
    async fn receiver(&self, mut eventloop: EventLoop) -> Result<()> {
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Incoming::Publish(publish))) => {
                    self.handle_message(publish)?;
                }
                Err(e) => {
                    // should automatically reconnect on next poll()..
                    error!("{}", e);
                }
                _ => {} // keepalives etc
            }
        }
    }

    fn handle_message(&self, publish: Publish) -> Result<()> {
        let message = Message {
            topic: publish.topic,
            payload: String::from_utf8(publish.payload.to_vec())?,
        };
        debug!("RX: {:?}", message);
        self.to_coordinator.send(message)?;

        Ok(())
    }

    // coordinator -> mqtt
    async fn sender(&self, client: AsyncClient) -> Result<()> {
        let mut receiver = self.from_coordinator.subscribe();
        loop {
            let message = receiver.recv().await?;
            debug!("publishing: {} = {}", message.topic, message.payload);
            client
                .publish(message.topic, QoS::AtLeastOnce, false, message.payload)
                .await?;
        }
    }
}
