module rsx-glass

go 1.25.0

require (
	golang.org/x/image v0.44.0
	rsx-term v0.0.0
)

require golang.org/x/text v0.40.0 // indirect

replace rsx-term => ../rsx-term
