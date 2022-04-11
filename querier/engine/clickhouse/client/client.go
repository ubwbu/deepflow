package client

import (
	"fmt"
	_ "github.com/ClickHouse/clickhouse-go"
	"github.com/jmoiron/sqlx"
	//"github.com/k0kubun/pp"
	logging "github.com/op/go-logging"
	"math/rand"
	"time"
)

var log = logging.MustGetLogger("clickhouse.client")

type Client struct {
	IPs        []string
	Port       int
	UserName   string
	Password   string
	connection *sqlx.DB
	DB         string
	Debug      *Debug
}

func (c *Client) Init(query_uuid string) error {
	if c.Debug == nil {
		c.Debug = &Debug{QueryUUID: query_uuid}
	}
	rand.Seed(time.Now().Unix())
	randIndex := rand.Intn(len(c.IPs))
	c.Debug.IP = c.IPs[randIndex]
	url := fmt.Sprintf("tcp://%s:%d?username=%s&password=%s&query_id=%s", c.IPs[randIndex], c.Port, c.UserName, c.Password, query_uuid)
	if c.DB != "" {
		url = fmt.Sprintf("%s&database=%s", url, c.DB)
	}
	conn, err := sqlx.Open(
		"clickhouse", url,
	)
	if err != nil {
		log.Errorf("connect clickhouse failed: %s, url: %s, query_uuid: %s", err, url, query_uuid)
		return err
	}
	c.connection = conn
	return nil
}

func (c *Client) DoQuery(sql string) (map[string][]interface{}, error) {
	start := time.Now()
	rows, err := c.connection.Queryx(sql)
	queryTime := time.Since(start)
	c.Debug.Sql = sql
	c.Debug.QueryTime = int64(queryTime)
	if err != nil {
		log.Errorf("query clickhouse Error: %s, sql: %s, query_uuid: %s", err, sql, c.Debug.QueryUUID)
		return nil, err
	}
	columns, err := rows.ColumnTypes()
	if err != nil {
		return nil, err
	}
	result := make(map[string][]interface{})
	var columnNames []interface{}
	var columnTypes []string
	// 获取列名和列类型
	for _, column := range columns {
		columnNames = append(columnNames, column.Name())
		columnTypes = append(columnTypes, column.DatabaseTypeName())
	}
	result["columns"] = columnNames
	for rows.Next() {
		row, err := rows.SliceScan()
		if err != nil {
			return nil, err
		}
		var values []interface{}
		for i, rawValue := range row {
			value, err := TransType(columnTypes[i], rawValue)
			if err != nil {
				return nil, err
			}
			//TODO: callback
			values = append(values, value)
		}
		result["values"] = append(result["values"], values)
	}
	return result, nil
}
