// Substitute an authenticated principal configured by the operator, never model-supplied identity.
@id("date-reader-clock")
permit(principal == Dekopon::Principal::"date-reader",
       action == Dekopon::Action::"date.now",
       resource == Dekopon::Provider::"date");
