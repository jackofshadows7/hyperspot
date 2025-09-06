This guideline defines the path for a new HyperSpot module creation

# STEP #1
- define API specification for the module
- define all the data types - DTO, model, storage
- implement DB migrations

# STEP #2
- implement basic CRUD logic
- implement appropriate auth check
- implement proper error handling and status reporting for the CRUD
- implement paging, filtering, sorting if needed

# STEP #3
- implement custom service logic, whatever is beyond CRUD

# STEP #4
- implement out-of-process execution via gRPC
