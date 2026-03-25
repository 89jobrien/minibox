package pubsub

import (
	"encoding/json"

	"github.com/joe/minibox/agentbox/internal/domain"
)

// MarshalMessage serializes a Message to a single JSONL line.
func MarshalMessage(msg domain.Message) ([]byte, error) {
	return json.Marshal(msg)
}

// UnmarshalMessage deserializes a JSONL line into a Message.
func UnmarshalMessage(data []byte) (domain.Message, error) {
	var msg domain.Message
	err := json.Unmarshal(data, &msg)
	return msg, err
}
