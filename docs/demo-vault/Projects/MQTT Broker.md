# MQTT Broker

Mosquitto runs on the Pi and relays every reading.

- Topic layout: `garden/<plant>/moisture`
- Retained messages so the dashboard has a value on startup
- No cloud: the broker only listens on the home network

Consumers: [[Data Dashboard]]. Producer: [[Raspberry Pi Setup]].

#project #network
