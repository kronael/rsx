module rsx-glass

go 1.25.0

require (
	github.com/coder/websocket v1.8.15
	golang.org/x/image v0.44.0
	rsx-term v0.0.0
)

require golang.org/x/text v0.40.0 // indirect

replace rsx-term => ../rsx-term
